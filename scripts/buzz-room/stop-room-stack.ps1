# Stop room processes started by start-room-stack.ps1 (bony-build only).
$ErrorActionPreference = "Continue"
. (Join-Path $PSScriptRoot "_paths.ps1")
$BonyRoot = Get-BonyRoot
$RuntimeDir = Join-Path $BonyRoot ".local-runtime"

foreach ($name in @("buzz-relay.pid", "grok-agent.pid")) {
  $f = Join-Path $RuntimeDir $name
  if (Test-Path $f) {
    $id = Get-Content $f -ErrorAction SilentlyContinue
    if ($id) {
      Write-Host "stop pid $id ($name)"
      Stop-Process -Id ([int]$id) -Force -ErrorAction SilentlyContinue
    }
    Remove-Item $f -Force -ErrorAction SilentlyContinue
  }
}

Get-Process buzz-relay, buzz-acp -ErrorAction SilentlyContinue | ForEach-Object {
  Write-Host "stop $($_.ProcessName) pid=$($_.Id)"
  Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
}

Write-Host "Room stack processes stopped. Docker infra left running (use docker compose down in third_party/buzz if needed)."
