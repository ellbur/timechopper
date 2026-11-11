# Focus moves between Notepad and the watcher's terminal: expect Notepad's
# exe and command line (with its file argument).
. (Join-Path $PSScriptRoot 'common.ps1')

$rec = $null; $w = $null; $np = $null
try {
    $rec = Start-Recording 'notepad'
    $w = Start-Watcher 20

    $file = Join-Path $env:RUNNER_TEMP 'timechopper-smoke.txt'
    Set-Content $file 'hello from the timechopper smoke test'
    $np = Start-Process notepad.exe -ArgumentList "`"$file`"" -PassThru
    Start-Sleep -Seconds 3
    $np.Refresh()
    $hwnd = $np.MainWindowHandle
    Set-WindowHalf $hwnd 'Right'
    Set-Foreground $hwnd $np.Id
    Set-Foreground $w.Hwnd $w.Title
    Set-Foreground $hwnd $np.Id

    $out = Wait-Watcher $w
}
finally {
    Stop-Recording $rec
    Close-Watcher $w
    if ($np) { Stop-Process -Id $np.Id -ErrorAction SilentlyContinue }
}

Assert-Match $out '(?i)exe:\s+[^\r\n]*\\notepad\.exe' 'notepad.exe'
Assert-Match $out 'timechopper-smoke\.txt' 'notepad command line'
Write-Host 'Notepad smoke test passed'
