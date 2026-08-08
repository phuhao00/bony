# Start optional Docker compose deps for local Buzz (MinIO, mesh Redis, …).
# Single-instance deployment needs none of this by default: persistence is
# SQLite, pub/sub is in-process. Docker is only touched when the caller (via
# .env COMPOSE_PROFILES or an explicit profile) actually wants MinIO and/or
# the opt-in buzz-relay-mesh Redis.
param(
  [string]$BuzzRoot = ""
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
if (-not $BuzzRoot) {
  try { $BuzzRoot = Get-BuzzRoot } catch {
    & (Join-Path $PSScriptRoot "setup-buzz.ps1")
    $BuzzRoot = Get-BuzzRoot
  }
}

Set-Location $BuzzRoot

$envFile = Join-Path $BuzzRoot ".env"
$profiles = ""
if (Test-Path $envFile) {
  $line = Get-Content $envFile | Where-Object { $_ -match '^\s*COMPOSE_PROFILES=' } | Select-Object -Last 1
  if ($line -match '^\s*COMPOSE_PROFILES=(.*)$') { $profiles = $Matches[1].Trim().Trim('"') }
}
if (-not $profiles -and $env:COMPOSE_PROFILES) { $profiles = $env:COMPOSE_PROFILES }

if (-not $profiles) {
  Write-Host "==> No Docker infra needed (single-instance: SQLite + in-process pub/sub)."
  Write-Host "    Set COMPOSE_PROFILES=minio and/or =mesh in .env to opt into bundled MinIO / mesh Redis."
  return
}

Write-Host "==> Checking Docker (COMPOSE_PROFILES=$profiles requested)..."
docker version --format "{{.Server.Version}}" | Out-Null
if ($LASTEXITCODE -ne 0) {
  throw "Docker engine not ready. Start Docker Desktop and retry, or clear COMPOSE_PROFILES in .env to skip Docker entirely."
}

Write-Host "==> docker compose --profile $profiles up -d"
docker compose --profile $profiles up -d

if ($profiles -match 'minio') {
  Write-Host "==> Waiting for MinIO..."
  $ok = $false
  for ($i = 0; $i -lt 30; $i++) {
    docker exec buzz-minio curl -sf http://localhost:9000/minio/health/live 2>$null | Out-Null
    if ($LASTEXITCODE -eq 0) { $ok = $true; break }
    Start-Sleep -Seconds 2
  }
  if (-not $ok) { Write-Warning "MinIO did not report healthy in time; continuing anyway." }
}

Write-Host "Infra is up."
