# Runs the watcher in its own visible console window so it shows up in the
# screen recording. Output is also teed to $Log for the test's assertions.
param(
    [string]$Exe,
    [int]$Seconds,
    [string]$Log,
    [string]$Done,
    [string]$Title
)
$host.UI.RawUI.WindowTitle = $Title
Write-Host "PS> timechopper --exit-after $Seconds" -ForegroundColor Cyan
& $Exe --exit-after $Seconds | Tee-Object -FilePath $Log
Write-Host '(watcher exited)' -ForegroundColor Cyan
New-Item -ItemType File $Done | Out-Null
# Keep the window open so the final output stays on screen; the test closes it.
Start-Sleep -Seconds 600
