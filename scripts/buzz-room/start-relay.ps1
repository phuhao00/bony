# Migrate DB (if needed) and start buzz-relay from the in-repo Buzz checkout.
param(
  [string]$BuzzRoot = "",
  [switch]$SkipMigrate
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
if (-not $BuzzRoot) { $BuzzRoot = Get-BuzzRoot }

Remove-Item Env:RUSTUP_TOOLCHAIN -ErrorAction SilentlyContinue
Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue

Set-Location $BuzzRoot
$envFile = Join-Path $BuzzRoot ".env"
if (-not (Test-Path $envFile) -and (Test-Path (Join-Path $BuzzRoot ".env.example"))) {
  Copy-Item (Join-Path $BuzzRoot ".env.example") $envFile
  Write-Host "Created .env from .env.example"
}
if (Test-Path $envFile) {
  Get-Content $envFile | ForEach-Object {
    if ($_ -match '^\s*#' -or $_ -match '^\s*$') { return }
    if ($_ -match '^\s*([^=]+)=(.*)$') {
      [Environment]::SetEnvironmentVariable($Matches[1].Trim(), $Matches[2].Trim().Trim('"'), 'Process')
    }
  }
}

$admin = Join-Path $BuzzRoot "target\debug\buzz-admin.exe"
if (-not (Test-Path $admin)) { $admin = Join-Path $BuzzRoot "target\release\buzz-admin.exe" }
$relay = Join-Path $BuzzRoot "target\debug\buzz-relay.exe"
if (-not (Test-Path $relay)) { $relay = Join-Path $BuzzRoot "target\release\buzz-relay.exe" }

if (-not (Test-Path $admin) -or -not (Test-Path $relay)) {
  Write-Host "==> Building buzz-admin + buzz-relay ..."
  cargo build -p buzz-admin -p buzz-relay
  if ($LASTEXITCODE -ne 0) { throw "build failed" }
  $admin = Join-Path $BuzzRoot "target\debug\buzz-admin.exe"
  $relay = Join-Path $BuzzRoot "target\debug\buzz-relay.exe"
}

if (-not $SkipMigrate) {
  Write-Host "==> migrate"
  & $admin migrate
  if ($LASTEXITCODE -ne 0) { throw "migrate failed" }
}

Write-Host "==> buzz-relay ($relay)"
Write-Host "    health: http://127.0.0.1:3000/health"
& $relay
