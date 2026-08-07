# One-shot: start Buzz room stack entirely from bony-build (third_party/buzz only).
# Does not reference C:\Users\Administrator\buzz or other external trees.
param(
  [switch]$SkipBuild,
  [switch]$SkipGrok,
  [switch]$ForegroundRelay
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
$BonyRoot = Get-BonyRoot
$BuzzRoot = Get-BuzzRoot
$RuntimeDir = Join-Path $BonyRoot ".local-runtime"
New-Item -ItemType Directory -Force -Path $RuntimeDir | Out-Null

function Import-DotEnv([string]$Path) {
  if (-not (Test-Path $Path)) { return }
  Get-Content $Path | ForEach-Object {
    if ($_ -match '^\s*#' -or $_ -match '^\s*$') { return }
    if ($_ -match '^\s*([^=]+)=(.*)$') {
      [Environment]::SetEnvironmentVariable($Matches[1].Trim(), $Matches[2].Trim().Trim('"'), 'Process')
    }
  }
}

function Start-WithEnv {
  param(
    [string]$FilePath,
    [string]$WorkingDirectory,
    [string]$StdoutLog,
    [string]$StderrLog,
    [hashtable]$ExtraEnv = @{}
  )
  $psi = New-Object System.Diagnostics.ProcessStartInfo
  $psi.FileName = $FilePath
  $psi.WorkingDirectory = $WorkingDirectory
  $psi.UseShellExecute = $false
  $psi.RedirectStandardOutput = $true
  $psi.RedirectStandardError = $true
  $psi.CreateNoWindow = $true
  # Inherit current process env (includes .env we loaded)
  foreach ($key in [System.Environment]::GetEnvironmentVariables('Process').Keys) {
    try { $psi.Environment[$key] = [System.Environment]::GetEnvironmentVariable($key, 'Process') } catch {}
  }
  foreach ($k in $ExtraEnv.Keys) { $psi.Environment[$k] = [string]$ExtraEnv[$k] }
  $p = New-Object System.Diagnostics.Process
  $p.StartInfo = $psi
  # async log pump
  $null = $p.Start()
  Start-Job -ScriptBlock {
    param($procId, $outPath, $errPath)
    $proc = Get-Process -Id $procId -ErrorAction SilentlyContinue
    if (-not $proc) { return }
    # Re-open via Process.GetProcessById doesn't give streams; write pid file only.
  } -ArgumentList $p.Id, $StdoutLog, $StderrLog | Out-Null
  return $p
}

Write-Host "==> Infra (Docker compose in $BuzzRoot)"
& (Join-Path $PSScriptRoot "start-infra.ps1") -BuzzRoot $BuzzRoot

if (-not $SkipBuild) {
  Write-Host "==> Build tools"
  & (Join-Path $PSScriptRoot "build-tools.ps1") -BonyRoot $BonyRoot -BuzzRoot $BuzzRoot
  # ensure relay
  if (-not (Test-Path (Join-Path $BuzzRoot "target\debug\buzz-relay.exe"))) {
    Set-Location $BuzzRoot
    Remove-Item Env:RUSTUP_TOOLCHAIN -ErrorAction SilentlyContinue
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    cargo build -p buzz-relay
    if ($LASTEXITCODE -ne 0) { throw "buzz-relay build failed" }
  }
}

$envFile = Join-Path $BuzzRoot ".env"
if (-not (Test-Path $envFile) -and (Test-Path (Join-Path $BuzzRoot ".env.example"))) {
  Copy-Item (Join-Path $BuzzRoot ".env.example") $envFile
}
Import-DotEnv $envFile
# Local monorepo: open rate limits (HTTP bridge uses human_api for all NIP-98 callers).
foreach ($pair in @(
  @('BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN','1000000'),
  @('BUZZ_RATE_LIMIT_HUMAN_API_CALLS_PER_MIN','1000000'),
  @('BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC','100000'),
  @('BUZZ_RATE_LIMIT_AGENT_STANDARD_MESSAGES_PER_MIN','1000000'),
  @('BUZZ_RATE_LIMIT_AGENT_STANDARD_API_CALLS_PER_MIN','1000000'),
  @('BUZZ_RATE_LIMIT_AGENT_ELEVATED_MESSAGES_PER_MIN','1000000'),
  @('BUZZ_RATE_LIMIT_AGENT_PLATFORM_MESSAGES_PER_MIN','1000000'),
  @('BUZZ_MAX_CONCURRENT_HANDLERS','8192')
)) {
  if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($pair[0], 'Process'))) {
    [Environment]::SetEnvironmentVariable($pair[0], $pair[1], 'Process')
  }
}
Write-Host "==> local rate limits opened (human/agent + concurrent handlers)"
Set-Location $BuzzRoot

function Find-RoomBin([string]$name) {
  foreach ($root in @($BonyRoot, $BuzzRoot)) {
    foreach ($prof in @("debug", "release")) {
      $p = Join-Path $root "target\$prof\$name.exe"
      if (Test-Path $p) { return $p }
    }
  }
  return $null
}

$admin = Find-RoomBin "buzz-admin"
$relay = Find-RoomBin "buzz-relay"
if (-not $relay) { throw "missing buzz-relay.exe under $BonyRoot\target or $BuzzRoot\target — run without -SkipBuild" }
if ($admin) {
  Write-Host "==> migrate via $admin"
  & $admin migrate
  if ($LASTEXITCODE -ne 0) { throw "migrate failed" }
} else {
  Write-Warning "buzz-admin.exe missing — skip migrate (existing docker DB assumed OK)"
}
Write-Host "==> using relay: $relay"

# relay pid file
$pidFile = Join-Path $RuntimeDir "buzz-relay.pid"
if (Test-Path $pidFile) {
  $old = Get-Content $pidFile -ErrorAction SilentlyContinue
  if ($old) { Stop-Process -Id ([int]$old) -Force -ErrorAction SilentlyContinue }
}
Get-Process buzz-relay -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

Write-Host "==> buzz-relay from $relay"
if ($ForegroundRelay) {
  & $relay
  exit $LASTEXITCODE
}

# Background: PowerShell inherits env; redirect logs (avoid Tee-Object + Hidden exit)
$runner = Join-Path $RuntimeDir "run-relay.ps1"
$relayLog = Join-Path $RuntimeDir "relay.log"
$relayErr = Join-Path $RuntimeDir "relay.err"
$runnerBody = @"
Set-Location '$BuzzRoot'
if (Test-Path '$envFile') {
  Get-Content '$envFile' | ForEach-Object {
    if (`$_ -match '^\s*#' -or `$_ -match '^\s*`$') { return }
    if (`$_ -match '^\s*([^=]+)=(.*)`$') {
      [Environment]::SetEnvironmentVariable(`$Matches[1].Trim(), `$Matches[2].Trim().Trim('"'), 'Process')
    }
  }
}
# Local room stack: no practical rate limits (same defaults as start-relay.ps1).
`$rl = @{
  BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN = '1000000'
  BUZZ_RATE_LIMIT_HUMAN_API_CALLS_PER_MIN = '1000000'
  BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC = '100000'
  BUZZ_RATE_LIMIT_AGENT_STANDARD_MESSAGES_PER_MIN = '1000000'
  BUZZ_RATE_LIMIT_AGENT_STANDARD_API_CALLS_PER_MIN = '1000000'
  BUZZ_RATE_LIMIT_AGENT_ELEVATED_MESSAGES_PER_MIN = '1000000'
  BUZZ_RATE_LIMIT_AGENT_PLATFORM_MESSAGES_PER_MIN = '1000000'
  BUZZ_MAX_CONCURRENT_HANDLERS = '8192'
}
foreach (`$k in `$rl.Keys) {
  if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable(`$k, 'Process'))) {
    [Environment]::SetEnvironmentVariable(`$k, `$rl[`$k], 'Process')
  }
}
& '$relay'
"@
[System.IO.File]::WriteAllText($runner, $runnerBody)

if (Test-Path $relayLog) { Remove-Item $relayLog -Force -ErrorAction SilentlyContinue }
if (Test-Path $relayErr) { Remove-Item $relayErr -Force -ErrorAction SilentlyContinue }

$relayProc = Start-Process -FilePath "powershell.exe" `
  -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $runner) `
  -WorkingDirectory $BuzzRoot -WindowStyle Hidden -PassThru `
  -RedirectStandardOutput $relayLog `
  -RedirectStandardError $relayErr
[System.IO.File]::WriteAllText($pidFile, "$($relayProc.Id)")
Write-Host "    relay wrapper pid=$($relayProc.Id)  log=$relayLog"

Write-Host "==> waiting health http://127.0.0.1:3000/health"
$ok = $false
for ($i = 0; $i -lt 60; $i++) {
  try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:3000/health" -UseBasicParsing -TimeoutSec 2
    if ($r.StatusCode -eq 200) { $ok = $true; break }
  } catch {}
  Start-Sleep -Seconds 2
}
if (-not $ok) {
  Write-Host "relay log tail:"
  Get-Content (Join-Path $RuntimeDir "relay.log") -Tail 30 -ErrorAction SilentlyContinue
  throw "relay health not ready"
}
Write-Host "    health OK"

# Room agents (Grok/ZeroClaw/Unity/OpenMontage/DocSmith) are no longer
# minted here as external buzz-acp processes: they're native Desktop
# managed-agents, seeded idempotently by Desktop itself on launch (native
# `seed_room_agents` Tauri command — see start-desktop.ps1). This script only
# brings up the relay/infra; run start-desktop.ps1 to get the agents.
if (-not $SkipGrok) {
  Write-Host "==> Room agents: seeded natively by Desktop on launch (run start-desktop.ps1)"
}

Write-Host ""
Write-Host "Stack ready (all under $BonyRoot):"
Write-Host "  BuzzRoot: $BuzzRoot"
Write-Host "  Relay:    http://127.0.0.1:3000/health"
Write-Host "  WS:       ws://localhost:3000"
Write-Host "  Logs:     $RuntimeDir"
Write-Host "  Next:     powershell -File scripts/buzz-room/start-desktop.ps1  (seeds + starts the 5 room agents)"
Write-Host "  Stop:     powershell -File scripts/buzz-room/stop-room-stack.ps1"
