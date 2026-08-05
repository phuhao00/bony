# Start Docker compose deps for local Buzz (Postgres, Redis, MinIO, …).
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

Write-Host "==> Checking Docker..."
docker version --format "{{.Server.Version}}" | Out-Null
if ($LASTEXITCODE -ne 0) {
  throw "Docker engine not ready. Start Docker Desktop and retry."
}

Set-Location $BuzzRoot
Write-Host "==> docker compose up -d (postgres redis minio …)"
docker compose up -d

Write-Host "==> Waiting for Postgres..."
$ok = $false
for ($i = 0; $i -lt 60; $i++) {
  docker exec buzz-postgres pg_isready -U buzz 2>$null | Out-Null
  if ($LASTEXITCODE -eq 0) { $ok = $true; break }
  Start-Sleep -Seconds 2
}
if (-not $ok) { throw "Postgres did not become ready." }

Write-Host "Infra is up."
Write-Host "Next: powershell -File scripts/buzz-room/start-relay.ps1"
Write-Host "Relay default: ws://localhost:3000"
