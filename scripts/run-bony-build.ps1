# Kill previous Bony Build instances, rebuild release, launch once.
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

Write-Host "Stopping previous bony-build processes..."
Get-Process -Name "bony-build","Bony Build" -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "  stop pid=$($_.Id)"
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
}
Start-Sleep -Milliseconds 400

Write-Host "Building release..."
cargo build -p bony-build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$candidates = @(
    (Join-Path $env:CARGO_TARGET_DIR "release\bony-build.exe"),
    "C:\Users\HHaou\AppData\Local\Temp\cursor-sandbox-cache\bee13f0b55b00f06e5f836f62dea999a\cargo-target\release\bony-build.exe",
    (Join-Path $Root "target\release\bony-build.exe")
) | Where-Object { $_ -and (Test-Path $_) }

$exe = $candidates | Select-Object -First 1
if (-not $exe) {
    Write-Error "bony-build.exe not found after build"
    exit 1
}

Write-Host "Launching $exe"
Start-Process -FilePath $exe -WorkingDirectory $Root
