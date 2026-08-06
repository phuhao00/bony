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

# Local monorepo room stack: effectively disable rate limits so Desktop
# (get_channels storms) + 4 external agents + auto-post don't stall UI sends.
# Requires positive integers (0 rejects). Override by exporting real values first.
function Set-LocalRateLimitDefaults {
  param([hashtable]$Map)
  foreach ($k in $Map.Keys) {
    $cur = [Environment]::GetEnvironmentVariable($k, 'Process')
    if ([string]::IsNullOrWhiteSpace($cur)) {
      [Environment]::SetEnvironmentVariable($k, [string]$Map[$k], 'Process')
    }
  }
}
Set-LocalRateLimitDefaults @{
  BUZZ_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN          = '1000000'
  BUZZ_RATE_LIMIT_HUMAN_API_CALLS_PER_MIN         = '1000000'
  BUZZ_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC         = '100000'
  BUZZ_RATE_LIMIT_AGENT_STANDARD_MESSAGES_PER_MIN = '1000000'
  BUZZ_RATE_LIMIT_AGENT_STANDARD_API_CALLS_PER_MIN= '1000000'
  BUZZ_RATE_LIMIT_AGENT_ELEVATED_MESSAGES_PER_MIN = '1000000'
  BUZZ_RATE_LIMIT_AGENT_PLATFORM_MESSAGES_PER_MIN = '1000000'
  BUZZ_MAX_CONCURRENT_HANDLERS                    = '8192'
}
Write-Host "    local rate limits: human msg/api/ws + agent limits effectively open"

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
