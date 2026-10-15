$ErrorActionPreference = 'Stop'
$Destination = __DEST__
$Stage = __STAGE__
$Backup = Join-Path $Stage 'previous.exe'
$Moved = $false
$Installed = $false
$Stopped = $false
try {
    $Old = Get-Process -Id __PID__ -ErrorAction SilentlyContinue
    if ($Old -and -not $Old.WaitForExit(120000)) { throw 'TTREE did not exit; update was cancelled.' }
    $Stopped = $true
    Move-Item -LiteralPath $Destination -Destination $Backup
    $Moved = $true
    Move-Item -LiteralPath (Join-Path $Stage 'TTREE.exe') -Destination $Destination
    $Installed = $true
    Start-Process -FilePath $Destination -ArgumentList '--show'
} catch {
    $Failure = $_.Exception.Message
    try {
        if ($Installed) { Move-Item -LiteralPath $Destination -Destination (Join-Path $Stage 'failed.exe') }
        if ($Moved) { Move-Item -LiteralPath $Backup -Destination $Destination }
        if ($Stopped -and (Test-Path -LiteralPath $Destination)) {
            Start-Process -FilePath $Destination -ArgumentList '--show'
        }
    } catch {
        $Failure += "`nRollback error: $($_.Exception.Message). Backup: $Backup"
    }
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.MessageBox]::Show("Update failed. Previous version restored where possible.`n$Failure", 'TTREE') | Out-Null
    exit 1
}
# PowerShell may still hold its own script open; cleanup is best effort.
Remove-Item -LiteralPath $Stage -Recurse -Force -ErrorAction SilentlyContinue
