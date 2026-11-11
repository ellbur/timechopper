# Focus moves to Calculator, a UWP app whose top-level window belongs to
# ApplicationFrameHost.exe: expect the host to be reported and the real app
# process to be resolved.
. (Join-Path $PSScriptRoot 'common.ps1')

$pkg = Get-AppxPackage Microsoft.WindowsCalculator
if (-not $pkg) {
    Write-Host 'Calculator not installed; installing from the Microsoft Store via winget'
    winget install --id 9WZDNCRFHVN5 --source msstore --exact --silent `
        --accept-package-agreements --accept-source-agreements --disable-interactivity |
        Select-Object -Last 3
    $pkg = Get-AppxPackage Microsoft.WindowsCalculator
    if (-not $pkg) { throw 'Calculator could not be installed' }
}
Write-Host "Calculator package: $($pkg.PackageFullName)"

$rec = $null; $w = $null
try {
    $rec = Start-Recording 'calculator'
    $w = Start-Watcher 25
    explorer.exe "shell:AppsFolder\$($pkg.PackageFamilyName)!App"
    $hwnd = Wait-Window 'ApplicationFrameWindow' 'Calculator' 30
    Start-Sleep -Seconds 2
    Set-WindowHalf $hwnd 'Right'
    Set-Foreground $hwnd 'Calculator'
    Set-Foreground $w.Hwnd $w.Title
    Set-Foreground $hwnd 'Calculator'

    Write-Host '--- --once while Calculator is focused ---'
    $once = (& $Exe --once | Out-String)
    Write-Host $once

    $out = Wait-Watcher $w
}
finally {
    Stop-Recording $rec
    Close-Watcher $w
    Get-Process CalculatorApp, Calculator -ErrorAction SilentlyContinue | Stop-Process -Force
}

Assert-Match $out 'class:\s+ApplicationFrameWindow' 'the UWP frame window'
Assert-Match $out '(?i)host:\s+pid \d+, [^\r\n]*\\ApplicationFrameHost\.exe' 'ApplicationFrameHost as host'
Assert-Match $out '(?i)exe:\s+[^\r\n]*\\WindowsApps\\Microsoft\.WindowsCalculator_[^\\]+\\Calculator(App)?\.exe' 'the Calculator app process'
Assert-Match $once '(?i)exe:\s+[^\r\n]*\\WindowsApps\\Microsoft\.WindowsCalculator_[^\\]+\\Calculator(App)?\.exe' 'the Calculator app process (--once)'
Write-Host 'Calculator smoke test passed'
