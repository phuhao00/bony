# Start ZeroClaw specialist behind buzz-acp (mentions-only).
param(
  [string]$BuzzRoot = "",
  [string]$RelayUrl = "ws://localhost:3000",
  [string]$ZeroclawBin = "$env:USERPROFILE\.bony-build\zeroclaw\target\release\zeroclaw.exe",
  [string]$KeysFile = (Join-Path $PSScriptRoot "keys\zeroclaw.json"),
  [string]$SystemPromptFile = (Join-Path $PSScriptRoot "prompts\zeroclaw-specialist.md")
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
if (-not $BuzzRoot) { $BuzzRoot = Get-BuzzRoot }

function Find-Bin([string]$name, [string]$root) {
  foreach ($prof in @("debug", "release")) {
    $p = Join-Path $root "target\$prof\$name.exe"
    if (Test-Path $p) { return $p }
  }
  return $null
}
$acp = Find-Bin "buzz-acp" $BuzzRoot
if (-not $acp) { throw "buzz-acp missing — build with scripts/buzz-room/build-tools.ps1" }
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

Write-Host "ZeroClaw specialist → $ZeroclawBin"
Write-Host "  buzz: $BuzzRoot"
& $acp
