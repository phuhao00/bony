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
Set-Location $BuzzRoot

$admin = Join-Path $BuzzRoot "target\debug\buzz-admin.exe"
$relay = Join-Path $BuzzRoot "target\debug\buzz-relay.exe"
if (-not (Test-Path $admin)) { throw "missing $admin — run without -SkipBuild" }
if (-not (Test-Path $relay)) { throw "missing $relay — run without -SkipBuild" }

Write-Host "==> migrate via $admin"
& $admin migrate
if ($LASTEXITCODE -ne 0) { throw "migrate failed" }

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

# Background: PowerShell process that inherits env from this script
$runner = Join-Path $RuntimeDir "run-relay.ps1"
@"
Set-Location '$BuzzRoot'
Get-Content '$envFile' | ForEach-Object {
  if (`$_ -match '^\s*#' -or `$_ -match '^\s*`$') { return }
  if (`$_ -match '^\s*([^=]+)=(.*)`$') {
    [Environment]::SetEnvironmentVariable(`$Matches[1].Trim(), `$Matches[2].Trim().Trim('"'), 'Process')
  }
}
& '$relay' *>&1 | Tee-Object -FilePath '$RuntimeDir\relay.log'
"@ | Set-Content -Encoding utf8 $runner

$relayProc = Start-Process -FilePath "powershell.exe" `
  -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $runner) `
  -WorkingDirectory $BuzzRoot -WindowStyle Hidden -PassThru
$relayProc.Id | Set-Content $pidFile
Write-Host "    relay wrapper pid=$($relayProc.Id)  log=$RuntimeDir\relay.log"

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

if (-not $SkipGrok) {
  $keys = Join-Path $PSScriptRoot "keys\grok.json"
  if (-not (Test-Path $keys)) {
    Write-Host "==> mint keys"
    & (Join-Path $PSScriptRoot "mint-agent-keys.ps1") -BuzzRoot $BuzzRoot
  }
  Write-Host "==> Grok coordinator (background)"
  $grokRunner = Join-Path $RuntimeDir "run-grok.ps1"
  $startGrok = Join-Path $PSScriptRoot "start-grok-agent.ps1"
  @"
& '$startGrok' *>&1 | Tee-Object -FilePath '$RuntimeDir\grok-agent.log'
"@ | Set-Content -Encoding utf8 $grokRunner
  $grokProc = Start-Process -FilePath "powershell.exe" `
    -ArgumentList @("-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $grokRunner) `
    -WorkingDirectory $BonyRoot -WindowStyle Hidden -PassThru
  $grokProc.Id | Set-Content (Join-Path $RuntimeDir "grok-agent.pid")
  Write-Host "    grok wrapper pid=$($grokProc.Id)  log=$RuntimeDir\grok-agent.log"
  Start-Sleep -Seconds 5
  if ($grokProc.HasExited) {
    Write-Warning "Grok agent exited early (code=$($grokProc.ExitCode)). See log."
    Get-Content (Join-Path $RuntimeDir "grok-agent.log") -Tail 40 -ErrorAction SilentlyContinue
  } else {
    Write-Host "    Grok harness running"
  }
}

Write-Host "==> Register room agents for Desktop visibility"
try {
  & (Join-Path $PSScriptRoot "register-room-agents.ps1") -RelayHttp "http://localhost:3000" -RelayWs "ws://localhost:3000"
} catch {
  Write-Warning "register-room-agents failed: $_"
}

Write-Host ""
Write-Host "Stack ready (all under $BonyRoot):"
Write-Host "  BuzzRoot: $BuzzRoot"
Write-Host "  Relay:    http://127.0.0.1:3000/health"
Write-Host "  WS:       ws://localhost:3000"
Write-Host "  Logs:     $RuntimeDir"
Write-Host "  Stop:     powershell -File scripts/buzz-room/stop-room-stack.ps1"
