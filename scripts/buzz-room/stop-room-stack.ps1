# Stop room processes started by start-room-stack / start-external-room-agents.
$ErrorActionPreference = "Continue"
. (Join-Path $PSScriptRoot "_paths.ps1")
$BonyRoot = Get-BonyRoot
$RuntimeDir = Join-Path $BonyRoot ".local-runtime"

function Stop-Tree {
  param([int]$ProcessId)
  if ($ProcessId -le 0) { return }
  Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object { $_.ParentProcessId -eq $ProcessId } |
    ForEach-Object { Stop-Tree -ProcessId ([int]$_.ProcessId) }
  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

foreach ($name in @(
    "buzz-relay.pid",
    "grok-agent.pid",
    "zeroclaw-agent.pid",
    "unity-agent.pid",
    "openmontage-agent.pid",
    "docsmith-agent.pid"
  )) {
  $f = Join-Path $RuntimeDir $name
  if (Test-Path $f) {
    $id = Get-Content $f -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($id -match '^\d+$') {
      Write-Host "stop tree pid $id ($name)"
      Stop-Tree -ProcessId ([int]$id)
    }
    Remove-Item $f -Force -ErrorAction SilentlyContinue
  }
}

foreach ($pn in @("buzz-relay", "buzz-acp", "zeroclaw", "grok")) {
  Get-Process -Name $pn -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "stop $($_.ProcessName) pid=$($_.Id)"
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
  }
}

Write-Host "Room stack processes stopped. Docker infra left running (use docker compose down in third_party/buzz if needed)."
