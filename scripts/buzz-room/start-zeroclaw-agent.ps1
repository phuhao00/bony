# Start ZeroClaw specialist behind buzz-acp (mentions-only).
param(
  [string]$BuzzRoot = "",
  [string]$BonyRoot = "",
  [string]$RelayUrl = "ws://localhost:3000",
  [string]$ZeroclawBin = "$env:USERPROFILE\.bony-build\zeroclaw\target\release\zeroclaw.exe",
  [string]$KeysFile = (Join-Path $PSScriptRoot "keys\zeroclaw.json"),
  [string]$SystemPromptFile = (Join-Path $PSScriptRoot "prompts\zeroclaw-specialist.md")
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
if (-not $BonyRoot) { $BonyRoot = Get-BonyRoot }
if (-not $BuzzRoot) { $BuzzRoot = Get-BuzzRoot }

function Find-Bin([string]$name, [string[]]$roots) {
  foreach ($r in $roots) {
    foreach ($prof in @("debug", "release")) {
      $p = Join-Path $r "target\$prof\$name.exe"
      if (Test-Path $p) { return $p }
    }
  }
  return $null
}
$acp = Find-Bin "buzz-acp" @($BonyRoot, $BuzzRoot)
if (-not $acp) { throw "buzz-acp missing — cargo build -p buzz-acp  (or build-tools.ps1)" }
if (-not (Test-Path $ZeroclawBin)) { throw "ZeroClaw binary not found: $ZeroclawBin" }

$nsec = $env:BUZZ_PRIVATE_KEY
if (-not $nsec -and (Test-Path $KeysFile)) {
  $k = Get-Content $KeysFile -Raw | ConvertFrom-Json
  if ($k.nsec) { $nsec = $k.nsec }
  elseif ($k.private_key) { $nsec = $k.private_key }
}
if (-not $nsec) { throw "Set BUZZ_PRIVATE_KEY or mint keys into $KeysFile" }

$env:BUZZ_RELAY_URL = $RelayUrl
$env:BUZZ_PRIVATE_KEY = $nsec
$env:BUZZ_ACP_AGENT_COMMAND = $ZeroclawBin
$env:BUZZ_ACP_AGENT_ARGS = "acp"
$env:BUZZ_ACP_SUBSCRIBE = "mentions"
$env:BUZZ_ACP_RESPOND_TO = "anyone"
$env:BUZZ_ACP_PERMISSION_MODE = "accept-edits"
$env:BUZZ_ACP_SYSTEM_PROMPT_FILE = $SystemPromptFile
$env:BUZZ_ACP_DISPLAY_NAME = "ZeroClaw"

. (Join-Path $PSScriptRoot "_agent-owner.ps1")
Set-RoomAgentOwner -BonyRoot $BonyRoot

# Specialists (ZeroClaw / weather tools) often stream a full answer without
# calling `buzz messages send`. With auto-post, buzz-acp publishes the stream
# buffer as a durable kind:9 so Desktop channel timelines show the reply.
$env:BUZZ_ACP_AUTO_POST_REPLY = "true"

# Load MaaS secrets if present (from .local-runtime/maas-llm.env)
$maasEnv = Join-Path (Get-BonyRoot) ".local-runtime\maas-llm.env"
if (Test-Path $maasEnv) {
  Get-Content $maasEnv | ForEach-Object {
    if ($_ -match '^\s*#' -or $_ -match '^\s*$') { return }
    if ($_ -match '^\s*([^=]+)=(.*)$') {
      $n = $Matches[1].Trim(); $v = $Matches[2].Trim().Trim('"')
      if ($n -and $v) { Set-Item -Path "Env:$n" -Value $v }
    }
  }
}

# DashScope / MaaS OpenAI-compatible — ensure key is visible to Hidden launches.
foreach ($k in @("DASHSCOPE_API_KEY", "QWEN_API_KEY", "OPENAI_API_KEY", "OPENAI_BASE_URL")) {
  if (-not [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($k, "Process"))) { continue }
  $u = [Environment]::GetEnvironmentVariable($k, "User")
  if ([string]::IsNullOrEmpty($u)) { $u = [Environment]::GetEnvironmentVariable($k, "Machine") }
  if (-not [string]::IsNullOrEmpty($u)) { Set-Item -Path "Env:$k" -Value $u }
}
if ([string]::IsNullOrEmpty($env:OPENAI_API_KEY) -and -not [string]::IsNullOrEmpty($env:DASHSCOPE_API_KEY)) {
  $env:OPENAI_API_KEY = $env:DASHSCOPE_API_KEY
}

Write-Host "ZeroClaw specialist → $ZeroclawBin"
Write-Host "  acp: $acp"
Write-Host "  buzz: $BuzzRoot"
if ($env:DASHSCOPE_API_KEY) {
  Write-Host "  llm key: set (len=$($env:DASHSCOPE_API_KEY.Length))"
  if ($env:OPENAI_BASE_URL) { Write-Host "  base: $($env:OPENAI_BASE_URL)" }
} else {
  Write-Warning "  llm key: MISSING — model calls will 401"
}
& $acp
