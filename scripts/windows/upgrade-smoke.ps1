param(
    [Parameter(Mandatory=$true)][string]$ProbeExe,
    [Parameter(Mandatory=$true)][string]$ReleaseDirectory,
    [Parameter(Mandatory=$true)][string]$ExpectedExe,
    [Parameter(Mandatory=$true)][string]$Version,
    [Parameter(Mandatory=$true)][string]$Output
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path "$PSScriptRoot/../..").Path
$Stage = Join-Path $Root "target/updater-smoke/portable user's app"
New-Item -ItemType Directory -Force $Stage, $Output | Out-Null
$Destination = Join-Path $Stage 'TTREE.exe'
Copy-Item -LiteralPath $ProbeExe -Destination $Destination -Force
Set-Content -LiteralPath (Join-Path $Stage 'local-settings.keep') -Value 'preserve me'
$Before = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash
$Expected = (Get-FileHash -LiteralPath $ExpectedExe -Algorithm SHA256).Hash
if ($Before -eq $Expected) { throw 'A and B must be different builds' }
$Source = (Resolve-Path $ReleaseDirectory).Path
# A damaged package must fail before any installed file is replaced.
$Tampered = Join-Path $Root 'target/updater-smoke/tampered-source'
New-Item -ItemType Directory -Force $Tampered | Out-Null
Copy-Item -Path (Join-Path $Source '*') -Destination $Tampered -Force
$Zip = Join-Path $Tampered "TTREE_${Version}_windows_x64.zip"
$File = [System.IO.File]::Open($Zip, [System.IO.FileMode]::Append)
try { $File.WriteByte(0) } finally { $File.Dispose() }
$Rejected = Start-Process -FilePath $Destination -ArgumentList @('--updater-smoke-install', ('"' + $Tampered + '"'), "v$Version") -PassThru
if (-not $Rejected.WaitForExit(30000)) { Stop-Process -Id $Rejected.Id -Force; throw 'Tampered package probe hung' }
if ($Rejected.ExitCode -eq 0) { throw 'Tampered package was accepted' }
if ((Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash -ne $Before) { throw 'Rejected update changed the installed executable' }
$Old = Start-Process -FilePath $Destination -ArgumentList @('--updater-smoke-install', ('"' + $Source + '"'), "v$Version") -PassThru
$New = $null
try {
    if (-not $Old.WaitForExit(120000)) { throw 'Installer probe did not exit' }
    if ($Old.ExitCode -ne 0) { throw "Installer probe failed: $($Old.ExitCode)" }
    for ($i = 0; $i -lt 120; $i++) {
        $New = Get-Process -Name TTREE -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Destination -and $_.Id -ne $Old.Id } | Select-Object -First 1
        if ($New) {
            $New.Refresh()
            if ($New.MainWindowHandle -ne [IntPtr]::Zero) { break }
        }
        Start-Sleep -Milliseconds 500
    }
    if (-not $New -or $New.MainWindowHandle -eq [IntPtr]::Zero) { throw 'Updated application did not restart with a native window' }
    Start-Sleep -Seconds 2
    $New.Refresh()
    if ($New.HasExited) { throw 'Updated application exited after restart' }
    if ((Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash -ne $Expected) { throw 'Installed executable does not match B' }
    if ((Get-Content -LiteralPath (Join-Path $Stage 'local-settings.keep')).Trim() -ne 'preserve me') { throw 'Local files changed during upgrade' }
    for ($i = 0; $i -lt 20; $i++) {
        if (-not (Get-ChildItem -LiteralPath $Stage -Force -Filter '.ttree-update-*')) { break }
        Start-Sleep -Milliseconds 500
    }
    if (Get-ChildItem -LiteralPath $Stage -Force -Filter '.ttree-update-*') { throw 'Installer did not finish cleanup' }
    @{ result='passed'; old_pid=$Old.Id; new_pid=$New.Id; old_sha256=$Before; installed_sha256=$Expected;
       checks=@('tampered package refused without replacement', 'pinned update signature', 'signed checksum', 'real executable replacement', 'new GUI process alive', 'portable local file preserved', 'helper cleanup') } |
        ConvertTo-Json | Set-Content (Join-Path $Output 'upgrade.json')
} finally {
    if (-not $Old.HasExited) { Stop-Process -Id $Old.Id -Force }
    if ($New -and -not $New.HasExited) { Stop-Process -Id $New.Id -Force }
}
