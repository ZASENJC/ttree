$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path "$PSScriptRoot/../..").Path
Push-Location $Root
try {
    $Version = python scripts/release/metadata.py
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read workspace version' }
    cargo build --locked --release -p ttree-windows --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
    $Stage = Join-Path $Root 'target/package/windows/TTREE'
    if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
    New-Item -ItemType Directory -Force $Stage, "$Root/dist/windows" | Out-Null
    Copy-Item target/x86_64-pc-windows-msvc/release/ttree-windows.exe "$Stage/TTREE.exe"
    Copy-Item README.md, LICENSE $Stage
    Copy-Item -Recurse assets/licenses "$Stage/licenses"
    Copy-Item apps/windows/README.md "$Stage/Windows.md"
    $Zip = "$Root/dist/windows/TTREE_${Version}_windows_x64.zip"
    Compress-Archive -Path $Stage -DestinationPath $Zip -Force
    # Validate the container and executable, not just cargo's exit code.
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $Archive = [System.IO.Compression.ZipFile]::OpenRead($Zip)
    try {
        if (-not ($Archive.Entries | Where-Object { $_.FullName -eq 'TTREE/TTREE.exe' -and $_.Length -gt 0 })) {
            throw 'Portable ZIP is missing TTREE/TTREE.exe'
        }
    } finally { $Archive.Dispose() }
    Write-Host "Windows portable build: $Zip"
} finally { Pop-Location }
