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
}
'@

$Exe = Join-Path $PSScriptRoot '..\target\release\timechopper.exe'

function Start-Watcher([int]$Seconds) {
    $log = Join-Path $env:RUNNER_TEMP ("watch-{0}.log" -f [guid]::NewGuid())
    $p = Start-Process -FilePath $Exe -ArgumentList '--exit-after', "$Seconds" `
        -RedirectStandardOutput $log -RedirectStandardError "$log.err" `
        -PassThru -NoNewWindow
    Start-Sleep -Seconds 2
    [pscustomobject]@{ Process = $p; Log = $log }
}

function Stop-Watcher($w) {
    $w.Process.WaitForExit()
    Write-Host '--- watcher stdout ---'
    Get-Content $w.Log | Write-Host
    Write-Host '--- watcher stderr ---'
    Get-Content "$($w.Log).err" | Write-Host
    Get-Content $w.Log -Raw
}

# Background processes can't always take focus; try both ways.
function Set-Foreground([IntPtr]$hwnd, $activateArg) {
    (New-Object -ComObject WScript.Shell).AppActivate($activateArg) | Out-Null
    [Win]::SetForegroundWindow($hwnd) | Out-Null
    Start-Sleep -Seconds 2
    Write-Host "target hwnd: $hwnd, foreground now: $([Win]::GetForegroundWindow())"
}

function Assert-Match($text, $pattern, $what) {
    if ($text -notmatch $pattern) { throw "watcher did not report $what (pattern: $pattern)" }
}
