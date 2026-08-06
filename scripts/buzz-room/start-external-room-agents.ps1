# Start Grok / ZeroClaw / Unity / OpenMontage as external buzz-acp processes
# (ws://localhost:3000). Avoids Desktop managed-agent setup-mode + 127.0.0.1 Host 404.
param(
  [string]$RelayUrl = "ws://localhost:3000",
  [switch]$SkipGrok,
  [switch]$SkipZeroClaw,
  [switch]$SkipUnity,
  [switch]$SkipOpenMontage,
  [switch]$RestartExisting  # kill prior wrappers before start
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
$BonyRoot = Get-BonyRoot
$RuntimeDir = Join-Path $BonyRoot ".local-runtime"
New-Item -ItemType Directory -Force -Path $RuntimeDir | Out-Null

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

function Start-RoomAgent {
  param(
    [string]$Name,
    [string]$StartScript,
    [string]$PidFileName,
    [string]$LogFileName
  )
  if (-not (Test-Path $StartScript)) {
    Write-Warning "missing $StartScript — skip $Name"
    return
  }
  $pidFile = Join-Path $RuntimeDir $PidFileName
  $logFile = Join-Path $RuntimeDir $LogFileName
  $errFile = Join-Path $RuntimeDir ($LogFileName -replace '\.log$', '.err')
  $runner = Join-Path $RuntimeDir ("run-$Name.ps1")

  if ($RestartExisting -and (Test-Path $pidFile)) {
    $old = (Get-Content $pidFile -ErrorAction SilentlyContinue | Select-Object -First 1)
    if ($old -match '^\d+$') {
      Write-Host "  stop old $Name wrapper pid=$old"
      Stop-Tree -ProcessId ([int]$old)
    }
    Remove-Item $pidFile -Force -ErrorAction SilentlyContinue
  } elseif (Test-Path $pidFile) {
    $old = (Get-Content $pidFile -ErrorAction SilentlyContinue | Select-Object -First 1)
    if ($old -match '^\d+$') {
      $alive = Get-Process -Id ([int]$old) -ErrorAction SilentlyContinue
      if ($alive) {
        Write-Host "  $Name already running (wrapper pid=$old) — skip"
        return
      }
    }
  }

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

  # Redirect, do NOT Tee-Object: Hidden windows + Tee often kill the wrapper after first line.
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
  }
}

Write-Host "==> External room agents on $RelayUrl"
if (-not $SkipGrok) {
  Start-RoomAgent -Name "grok" -StartScript (Join-Path $PSScriptRoot "start-grok-agent.ps1") `
    -PidFileName "grok-agent.pid" -LogFileName "grok-agent.log"
}
if (-not $SkipZeroClaw) {
  Start-RoomAgent -Name "zeroclaw" -StartScript (Join-Path $PSScriptRoot "start-zeroclaw-agent.ps1") `
    -PidFileName "zeroclaw-agent.pid" -LogFileName "zeroclaw-agent.log"
}
if (-not $SkipUnity) {
  Start-RoomAgent -Name "unity" -StartScript (Join-Path $PSScriptRoot "start-unity-agent.ps1") `
    -PidFileName "unity-agent.pid" -LogFileName "unity-agent.log"
}
if (-not $SkipOpenMontage) {
  Start-RoomAgent -Name "openmontage" -StartScript (Join-Path $PSScriptRoot "start-openmontage-agent.ps1") `
    -PidFileName "openmontage-agent.pid" -LogFileName "openmontage-agent.log"
}

Start-Sleep -Seconds 5

# Auto-route requires Grok (subscribe=all) to be a bot on every human channel
# that already has a room specialist; mesh membership quietly.
Write-Host "==> Room bot mesh (auto-route readiness)"
try {
  & (Join-Path $PSScriptRoot "ensure-room-bot-mesh.ps1") -RelayHttp (
    ($RelayUrl -replace '^wss://', 'https://' -replace '^ws://', 'http://')
  )
} catch {
  Write-Warning "ensure-room-bot-mesh failed: $_"
}

Write-Host ""
Write-Host "Done. buzz-acp processes:"
Get-Process buzz-acp -ErrorAction SilentlyContinue | Format-Table Id, StartTime, CPU -AutoSize

Write-Host "Ready checks (agent_pool_ready / subscribe):"
foreach ($n in @("grok", "zeroclaw", "unity", "openmontage")) {
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
  $status = if ($ready) { "READY" } elseif ($alivePid) { "UP (waiting ready)" } else { "DOWN" }
  Write-Host ("  {0,-12} {1}  wrapper={2}" -f $n, $status, $(if ($alivePid) { $alivePid } else { "-" }))
}

Write-Host "Logs: $RuntimeDir\*-agent.log"
Write-Host "Auto-route: Grok (subscribe=all) answers / delegates; specialists on @mention (or Grok @s them)."
Write-Host "Mesh ensures Grok+bots sit on channels room agents already use."
