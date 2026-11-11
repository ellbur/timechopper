# Focus moves to Notepad: expect its exe and command line (with file argument).
. (Join-Path $PSScriptRoot 'common.ps1')

$w = Start-Watcher 15
$file = Join-Path $env:RUNNER_TEMP 'timechopper-smoke.txt'
Set-Content $file 'hello'
$np = Start-Process notepad.exe -ArgumentList "`"$file`"" -PassThru
Start-Sleep -Seconds 3
$np.Refresh()
Set-Foreground $np.MainWindowHandle $np.Id
$out = Stop-Watcher $w
Stop-Process -Id $np.Id -ErrorAction SilentlyContinue

Assert-Match $out '(?i)exe:\s+\S*\\notepad\.exe' 'notepad.exe'
Assert-Match $out 'timechopper-smoke\.txt' 'notepad command line'
Write-Host 'Notepad smoke test passed'
