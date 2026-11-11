# Shared helpers for the Windows smoke tests.
$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Win {
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet = CharSet.Unicode)]
  public static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll")]
  public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int ht, bool repaint);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
}
'@

$Exe = (Resolve-Path (Join-Path $PSScriptRoot '..\target\release\timechopper.exe')).Path
$VideoDir = Join-Path $env:RUNNER_TEMP 'videos'
New-Item -ItemType Directory -Force $VideoDir | Out-Null

function Wait-Window([string]$cls, [string]$title, [int]$timeoutSec = 20) {
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    do {
        $h = [Win]::FindWindow($cls, $title)
        if ($h -ne [IntPtr]::Zero) { return $h }
        Start-Sleep -Milliseconds 250
    } while ((Get-Date) -lt $deadline)
    throw "window '$title' did not appear"
}

# Put a window on the left or right half of the screen.
function Set-WindowHalf([IntPtr]$hwnd, [ValidateSet('Left', 'Right')]$side) {
    $sw = [Win]::GetSystemMetrics(0); $sh = [Win]::GetSystemMetrics(1)
    $x = if ($side -eq 'Left') { 0 } else { [int]($sw / 2) }
    [Win]::MoveWindow($hwnd, $x, 0, [int]($sw / 2), $sh - 40, $true) | Out-Null
}

# Windows refuses SetForegroundWindow from a background process once
# something else holds the foreground. A synthetic Alt key press counts as
# input to this process and lifts that lock; retry until focus moves.
function Set-Foreground([IntPtr]$hwnd, $activateArg) {
    for ($i = 1; $i -le 5; $i++) {
        [Win]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero)   # Alt down
        [Win]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)   # Alt up
        (New-Object -ComObject WScript.Shell).AppActivate($activateArg) | Out-Null
        [Win]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 500
        if ([Win]::GetForegroundWindow() -eq $hwnd) {
            Write-Host "focused $hwnd (attempt $i)"
            Start-Sleep -Seconds 2   # time for the watcher (and the video) to show it
            return
        }
    }
    throw "could not move focus to $hwnd (foreground is $([Win]::GetForegroundWindow()))"
}

# --- Screen recording (ffmpeg gdigrab) ---

function Start-Recording([string]$name) {
    $psi = [System.Diagnostics.ProcessStartInfo]::new('ffmpeg')
    foreach ($a in @('-hide_banner', '-loglevel', 'error', '-y', '-f', 'gdigrab',
            '-framerate', '10', '-i', 'desktop',
            '-vf', 'scale=trunc(iw/2)*2:trunc(ih/2)*2',
            '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p',
            (Join-Path $VideoDir "$name.mp4"))) { $psi.ArgumentList.Add($a) }
    $psi.UseShellExecute = $false
    $psi.RedirectStandardInput = $true
    $psi.CreateNoWindow = $true
    $p = [System.Diagnostics.Process]::Start($psi)
    Start-Sleep -Seconds 1
    [pscustomobject]@{ Process = $p; Name = $name }
}

function Stop-Recording($rec) {
    if (-not $rec) { return }
    if (-not $rec.Process.HasExited) {
        $rec.Process.StandardInput.Write('q')   # lets ffmpeg finalize the mp4
        $rec.Process.StandardInput.Close()
        if (-not $rec.Process.WaitForExit(20000)) { $rec.Process.Kill() }
    }
    $mp4 = Join-Path $VideoDir "$($rec.Name).mp4"
    if (Test-Path $mp4) {
        # A contact sheet of stills, for a quick look without playing the video.
        ffmpeg -hide_banner -loglevel error -y -i $mp4 `
            -vf 'fps=1/2,scale=640:-2,tile=3x3' -frames:v 1 `
            (Join-Path $VideoDir "$($rec.Name)-frames.png")
        # Average brightness per second, as a sanity check that the capture
        # isn't black (YAVG near 16 means a blank frame).
        $yavg = ffmpeg -hide_banner -loglevel error -i $mp4 `
            -vf 'fps=1,signalstats,metadata=print:key=lavfi.signalstats.YAVG:file=-' -f null - |
            Select-String 'YAVG=([\d.]+)' | ForEach-Object { [math]::Round([double]$_.Matches[0].Groups[1].Value) }
        Write-Host "recorded $mp4 ($((Get-Item $mp4).Length) bytes); brightness per second: $($yavg -join ' ')"
    }
}

# --- Watcher in a visible terminal ---

function Start-Watcher([int]$Seconds) {
    $id = [guid]::NewGuid().ToString('N').Substring(0, 8)
    $log = Join-Path $env:RUNNER_TEMP "watch-$id.log"
    $done = "$log.done"
    $title = "timechopper watcher $id"
    $script = Join-Path $PSScriptRoot 'watch-in-terminal.ps1'
    # conhost.exe gives a classic console window we can find and position.
    $p = Start-Process conhost.exe -PassThru -ArgumentList @(
        'pwsh', '-NoLogo', '-NoProfile', '-File', $script,
        '-Exe', $Exe, '-Seconds', $Seconds, '-Log', $log, '-Done', $done, '-Title', "`"$title`"")
    $hwnd = Wait-Window 'ConsoleWindowClass' $title
    Set-WindowHalf $hwnd 'Left'
    Start-Sleep -Seconds 1
    [pscustomobject]@{ Process = $p; Log = $log; Done = $done; Hwnd = $hwnd; Seconds = $Seconds; Title = $title }
}

# Wait for the watcher to exit; return its output.
function Wait-Watcher($w) {
    $deadline = (Get-Date).AddSeconds($w.Seconds + 30)
    while (-not (Test-Path $w.Done)) {
        if ((Get-Date) -gt $deadline) { throw 'watcher did not finish' }
        Start-Sleep -Milliseconds 250
    }
    Start-Sleep -Seconds 2   # leave the final output on screen for the video
    $out = Get-Content $w.Log -Raw
    Write-Host '--- watcher output ---'
    Write-Host $out
    $out
}

function Close-Watcher($w) {
    if ($w) { taskkill /T /F /PID $w.Process.Id 2>&1 | Out-Null }
}

function Assert-Match($text, $pattern, $what) {
    if ($text -notmatch $pattern) { throw "watcher did not report $what (pattern: $pattern)" }
}
