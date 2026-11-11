# The live window: start it, move focus between Notepad and Calculator, and
# expect it to stay up, show them (see the screenshot and video) and keep
# them in its focus log.
. (Join-Path $PSScriptRoot 'common.ps1')

$log = Join-Path $env:RUNNER_TEMP 'timechopper-ui.log'
$err = Join-Path $env:RUNNER_TEMP 'timechopper-ui.err'
Remove-Item $log, $err -ErrorAction SilentlyContinue
$file = Join-Path $env:RUNNER_TEMP 'timechopper-ui.txt'
Set-Content $file 'hello from the live window smoke test'
$pkg = Get-AppxPackage Microsoft.WindowsCalculator

$rec = $null; $ui = $null; $np = $null
try {
    $rec = Start-Recording 'live-window'
    $ui = Start-Process $Exe -PassThru -RedirectStandardError $err `
        -ArgumentList @('ui', '--log-file', "`"$log`"", '--idle', '60')
    Wait-Window '' 'timechopper' 30 | Out-Null
    Start-Sleep -Seconds 2

    $np = Start-Process notepad.exe -ArgumentList "`"$file`"" -PassThru
    Start-Sleep -Seconds 3
    $np.Refresh()
    $notepad = $np.MainWindowHandle
    Set-WindowHalf $notepad 'Left'
    explorer.exe "shell:AppsFolder\$($pkg.PackageFamilyName)!App"
    $calc = Wait-Window 'ApplicationFrameWindow' 'Calculator' 30
    Start-Sleep -Seconds 2
    Set-WindowHalf $calc 'Right'

    foreach ($i in 1..2) {
        Set-Foreground $notepad $np.Id
        Start-Sleep -Seconds 3
        Set-Foreground $calc 'Calculator'
        Start-Sleep -Seconds 3
    }
    Set-Foreground $notepad $np.Id
    Start-Sleep -Seconds 3

    $shot = Join-Path $VideoDir 'live-window.png'
    ffmpeg -hide_banner -loglevel error -y -f gdigrab -i desktop -frames:v 1 $shot
    if ($ui.HasExited) { throw "the live window exited early (code $($ui.ExitCode))" }
}
finally {
    Stop-Recording $rec
    if ($ui -and -not $ui.HasExited) { Stop-Process -Id $ui.Id -Force }
    if ($np) { Stop-Process -Id $np.Id -ErrorAction SilentlyContinue }
    Get-Process CalculatorApp, Calculator -ErrorAction SilentlyContinue | Stop-Process -Force
    if (Test-Path $err) { Write-Host '--- stderr ---'; Get-Content $err | Select-Object -Last 40 }
}

$out = Get-Content $log -Raw
Write-Host '--- focus log ---'
Write-Host $out
if ($out -notmatch '(?m)^\d\d:\d\d:\d\d notepad\.exe timechopper-ui\.txt') { throw 'Notepad missing from the focus log' }
if ($out -notmatch '(?m)^\d\d:\d\d:\d\d Calculator(App)?\.exe Calculator') { throw 'Calculator missing from the focus log' }
$switches = ([regex]::Matches($out, '(?m)^\d\d:\d\d:\d\d notepad\.exe')).Count
if ($switches -lt 3) { throw "expected Notepad in focus at least 3 times, got $switches" }
Write-Host 'Live window smoke test passed'
