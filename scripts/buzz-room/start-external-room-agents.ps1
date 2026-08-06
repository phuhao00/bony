# Start every room agent that has keys under scripts/buzz-room/keys/
# (Grok / ZeroClaw / Unity / OpenMontage). Guarantees a single instance each —
# no orphan buzz-acp and no "already running" skip that leaves others offline.
param(
  [string]$RelayUrl = "ws://localhost:3000",
  # Opt-out only: by default always kill orphans/wrappers before start (no duplicates).
  [switch]$KeepExisting,
  # Legacy: treated same as default (ensure single). Kept so old call sites still work.
  [switch]$RestartExisting,
  [switch]$EnsureSingle,
  [switch]$SkipGrok,
  [switch]$SkipZeroClaw,
  [switch]$SkipUnity,
  [switch]$SkipOpenMontage,
  [switch]$SkipDocSmith
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
$BonyRoot = Get-BonyRoot
$RuntimeDir = Join-Path $BonyRoot ".local-runtime"
$KeysDir = Join-Path $PSScriptRoot "keys"
New-Item -ItemType Directory -Force -Path $RuntimeDir | Out-Null

if ($RestartExisting) { } # no-op alias: default is already single-instance
if ($EnsureSingle) { } # no-op alias: default is already single-instance

# Canonical host: relay multi-tenant is localhost:3000, not 127.0.0.1
if ($RelayUrl -match '://127\.0\.0\.1') {
  $RelayUrl = $RelayUrl -replace '://127\.0\.0\.1', '://localhost'
  Write-Host "normalized RelayUrl -> $RelayUrl"
}

function Stop-Tree {
  param([int]$ProcessId)
  if ($ProcessId -le 0) { return }
  Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object { $_.ParentProcessId -eq $ProcessId } |
    ForEach-Object { Stop-Tree -ProcessId ([int]$_.ProcessId) }
  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Stop-AllRoomAgentProcesses {
  Write-Host "==> Stop room agent processes (single-instance)"
  foreach ($name in @("grok", "zeroclaw", "unity", "openmontage", "docsmith")) {
    $pidFile = Join-Path $RuntimeDir "$name-agent.pid"
    if (Test-Path $pidFile) {
      $old = (Get-Content $pidFile -ErrorAction SilentlyContinue | Select-Object -First 1)
      if ($old -match '^\d+$') {
        Write-Host "  stop $name wrapper pid=$old"
        Stop-Tree -ProcessId ([int]$old)
      }
      Remove-Item $pidFile -Force -ErrorAction SilentlyContinue
    }
  }
  # Orphans: direct buzz-acp / agent CLI without wrapper pid file.
  foreach ($pn in @("buzz-acp", "zeroclaw")) {
    Get-Process -Name $pn -ErrorAction SilentlyContinue | ForEach-Object {
      Write-Host "  stop orphan $($_.ProcessName) pid=$($_.Id)"
      Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
    }
  }
  # Grok CLI is used as the brain for multiple room seats — only kill grok.exe
  # after wrappers (children of dead wrappers stick around and fan out).
  Get-Process -Name "grok" -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Host "  stop orphan grok pid=$($_.Id)"
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
  }
  Start-Sleep -Seconds 2
}

function Test-AgentHasKey([string]$Id) {
  $kf = Join-Path $KeysDir "$Id.json"
  if (-not (Test-Path $kf)) { return $false }
  try {
    $k = Get-Content $kf -Raw | ConvertFrom-Json
    $sk = if ($k.nsec) { $k.nsec } elseif ($k.private_key) { $k.private_key } else { $null }
    return -not [string]::IsNullOrWhiteSpace($sk)
  } catch { return $false }
}

function Start-RoomAgent {
  param(
    [string]$Name,
    [string]$StartScript,
    [string]$PidFileName,
    [string]$LogFileName
  )
  if (-not (Test-Path $StartScript)) {
    Write-Warning "missing $StartScript — skip $Name"
    return $false
  }
  if (-not (Test-AgentHasKey $Name)) {
    Write-Warning "missing keys for $Name ($KeysDir\$Name.json) — skip"
    return $false
  }

  $pidFile = Join-Path $RuntimeDir $PidFileName
  $logFile = Join-Path $RuntimeDir $LogFileName
  $errFile = Join-Path $RuntimeDir ($LogFileName -replace '\.log$', '.err')
  $runner = Join-Path $RuntimeDir ("run-$Name.ps1")

  # ASCII only (no BOM) — PS 5.1 + UTF8 BOM on .ps1 can mis-parse under -File
  $runnerBody = @"
`$ErrorActionPreference = 'Continue'
`$env:Path = [System.Environment]::GetEnvironmentVariable('Path','Machine') + ';' + [System.Environment]::GetEnvironmentVariable('Path','User')
`$env:RUST_LOG = 'info,buzz_acp=info'
# Prefer User-level API keys (XAI / DashScope / etc.) even under Hidden launches
foreach (`$k in @(
  'XAI_API_KEY','GROK_API_KEY','OPENAI_API_KEY','ANTHROPIC_API_KEY',
  'DASHSCOPE_API_KEY','QWEN_API_KEY','ZEROCLAW_API_KEY','BUZZ_AGENT_PROVIDER'
)) {
  if (-not [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable(`$k, 'Process'))) { continue }
  `$u = [Environment]::GetEnvironmentVariable(`$k, 'User')
  if ([string]::IsNullOrEmpty(`$u)) { `$u = [Environment]::GetEnvironmentVariable(`$k, 'Machine') }
  if (-not [string]::IsNullOrEmpty(`$u)) { Set-Item -Path "Env:`$k" -Value `$u }
}
Remove-Item Env:BUZZ_PRIVATE_KEY -ErrorAction SilentlyContinue
Remove-Item Env:NOSTR_PRIVATE_KEY -ErrorAction SilentlyContinue
& '$StartScript' -RelayUrl '$RelayUrl'
"@
  [System.IO.File]::WriteAllText($runner, $runnerBody)

  if (Test-Path $logFile) { Remove-Item $logFile -Force -ErrorAction SilentlyContinue }
  if (Test-Path $errFile) { Remove-Item $errFile -Force -ErrorAction SilentlyContinue }

  $proc = Start-Process -FilePath "powershell.exe" `
    -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $runner) `
    -WindowStyle Hidden -PassThru `
    -WorkingDirectory $BonyRoot `
    -RedirectStandardOutput $logFile `
    -RedirectStandardError $errFile
  [System.IO.File]::WriteAllText($pidFile, "$($proc.Id)")
  Write-Host "  $Name wrapper pid=$($proc.Id)  log=$logFile"
  Start-Sleep -Seconds 3
  if ($proc.HasExited) {
    Write-Warning "  $Name exited early code=$($proc.ExitCode) — see $logFile / $errFile"
    if (Test-Path $logFile) { Get-Content $logFile -Tail 25 -ErrorAction SilentlyContinue }
    if (Test-Path $errFile) { Get-Content $errFile -Tail 25 -ErrorAction SilentlyContinue }
    return $false
  }
  return $true
}

Write-Host "==> External room agents on $RelayUrl (all seats with keys, single instance)"
if (-not $KeepExisting) {
  Stop-AllRoomAgentProcesses
}

# Order: specialists first then coordinator so mesh/publish is fresh;
# all four still end up up together.
$agents = @(
  @{ Name = "zeroclaw";    Skip = $SkipZeroClaw;    Script = "start-zeroclaw-agent.ps1" }
  @{ Name = "unity";       Skip = $SkipUnity;       Script = "start-unity-agent.ps1" }
  @{ Name = "openmontage"; Skip = $SkipOpenMontage; Script = "start-openmontage-agent.ps1" }
  @{ Name = "docsmith";    Skip = $SkipDocSmith;    Script = "start-docsmith-agent.ps1" }
  @{ Name = "grok";        Skip = $SkipGrok;        Script = "start-grok-agent.ps1" }
)

$started = @()
foreach ($a in $agents) {
  if ($a.Skip) {
    Write-Host "  $($a.Name) skipped by flag"
    continue
  }
  if (-not (Test-AgentHasKey $a.Name)) {
    Write-Warning "  $($a.Name): no key file — not in roster, skip"
    continue
  }
  $ok = Start-RoomAgent -Name $a.Name `
    -StartScript (Join-Path $PSScriptRoot $a.Script) `
    -PidFileName "$($a.Name)-agent.pid" `
    -LogFileName "$($a.Name)-agent.log"
  if ($ok) { $started += $a.Name }
}

Start-Sleep -Seconds 5

# Dedup hard: more buzz-acp than seats usually means duplicate start scripts.
$acpCount = @(Get-Process buzz-acp -ErrorAction SilentlyContinue).Count
$expected = $started.Count
if ($acpCount -gt $expected -and $expected -gt 0) {
  Write-Warning "buzz-acp count=$acpCount > started seats=$expected — reaping extras and restarting once"
  Stop-AllRoomAgentProcesses
  foreach ($name in $started) {
    $script = switch ($name) {
      "zeroclaw" { "start-zeroclaw-agent.ps1" }
      "unity" { "start-unity-agent.ps1" }
      "openmontage" { "start-openmontage-agent.ps1" }
      "docsmith" { "start-docsmith-agent.ps1" }
      "grok" { "start-grok-agent.ps1" }
    }
    $null = Start-RoomAgent -Name $name `
      -StartScript (Join-Path $PSScriptRoot $script) `
      -PidFileName "$name-agent.pid" `
      -LogFileName "$name-agent.log"
  }
  Start-Sleep -Seconds 5
}

Write-Host "==> Room bot mesh (every seat in every channel that has any room bot)"
try {
  & (Join-Path $PSScriptRoot "ensure-room-bot-mesh.ps1") -RelayHttp (
    ($RelayUrl -replace '^wss://', 'https://' -replace '^ws://', 'http://')
  )
} catch {
  Write-Warning "ensure-room-bot-mesh failed: $_"
}

Write-Host ""
Write-Host "Done. buzz-acp processes (expect ~$($started.Count)):"
Get-Process buzz-acp -ErrorAction SilentlyContinue | Format-Table Id, StartTime, CPU -AutoSize

Write-Host "Seat readiness:"
foreach ($n in @("grok", "zeroclaw", "unity", "openmontage", "docsmith")) {
  $log = Join-Path $RuntimeDir "$n-agent.log"
  $err = Join-Path $RuntimeDir "$n-agent.err"
  $ready = $false
  if (Test-Path $log) {
    $ready = $null -ne (Select-String -Path $log -Pattern "agent_pool_ready" -SimpleMatch -ErrorAction SilentlyContinue | Select-Object -First 1)
  }
  if (-not $ready -and (Test-Path $err)) {
    $ready = $null -ne (Select-String -Path $err -Pattern "agent_pool_ready" -SimpleMatch -ErrorAction SilentlyContinue | Select-Object -First 1)
  }
  $alivePid = $null
  $pf = Join-Path $RuntimeDir "$n-agent.pid"
  if (Test-Path $pf) {
    $raw = (Get-Content $pf -ErrorAction SilentlyContinue | Select-Object -First 1)
    if ($raw -match '^\d+$' -and (Get-Process -Id ([int]$raw) -ErrorAction SilentlyContinue)) {
      $alivePid = $raw
    }
  }
  $hasKey = Test-AgentHasKey $n
  if (-not $hasKey) {
    Write-Host ("  {0,-12} NO_KEY" -f $n)
    continue
  }
  $status = if ($ready) { "READY" } elseif ($alivePid) { "UP (waiting ready)" } else { "DOWN" }
  Write-Host ("  {0,-12} {1}  wrapper={2}" -f $n, $status, $(if ($alivePid) { $alivePid } else { "-" }))
}

$finalAcp = @(Get-Process buzz-acp -ErrorAction SilentlyContinue).Count
Write-Host "buzz-acp count=$finalAcp (started seats: $($started -join ', '))"
if ($finalAcp -ne $started.Count -and $started.Count -gt 0) {
  Write-Warning "Expected $($started.Count) buzz-acp, got $finalAcp — check .local-runtime/*-agent.err"
}
Write-Host "Logs: $RuntimeDir\*-agent.log"
Write-Host "Policy: all keyed room agents start; EnsureSingle kills orphans before launch (no duplicates)."
