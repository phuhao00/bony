# Start OpenMontage specialist behind buzz-acp (mentions-only).
# Brain: Grok CLI unless BUZZ_AGENT_PROVIDER is set for buzz-agent.
param(
  [string]$BuzzRoot = "",
  [string]$BonyRoot = "",
  [string]$RelayUrl = "ws://localhost:3000",
  [string]$KeysFile = (Join-Path $PSScriptRoot "keys\openmontage.json"),
  [string]$SystemPromptFile = (Join-Path $PSScriptRoot "prompts\openmontage-specialist.md"),
  [string]$OpenMontageRoot = "$env:USERPROFILE\.bony-build\openmontage"
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
function Find-Grok {
  foreach ($c in @(
      (Join-Path $env:APPDATA "npm\grok.cmd"),
      (Join-Path $env:LOCALAPPDATA "npm\grok.cmd")
    )) {
    if (Test-Path $c) { return $c }
  }
  $cmd = Get-Command grok.cmd -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  return $null
}

$roots = @($BonyRoot, $BuzzRoot)
$acp = Find-Bin "buzz-acp" $roots
$mcp = Find-Bin "bony-room-tools-mcp" $roots
$agent = Find-Bin "buzz-agent" $roots
$grok = Find-Grok
if (-not $acp) { throw "buzz-acp missing" }
if (-not $mcp) { throw "bony-room-tools-mcp missing" }

$nsec = $env:BUZZ_PRIVATE_KEY
if (-not $nsec -and (Test-Path $KeysFile)) {
  $k = Get-Content $KeysFile -Raw | ConvertFrom-Json
  if ($k.nsec) { $nsec = $k.nsec }
  elseif ($k.private_key) { $nsec = $k.private_key }
}
if (-not $nsec) { throw "need nsec in env or $KeysFile" }

$env:BUZZ_RELAY_URL = $RelayUrl
$env:BUZZ_PRIVATE_KEY = $nsec
$env:BUZZ_ACP_MCP_COMMAND = $mcp
$env:BUZZ_ACP_SUBSCRIBE = "mentions"
$env:BUZZ_ACP_RESPOND_TO = "anyone"
$env:BUZZ_ACP_PERMISSION_MODE = "accept-edits"
$env:BUZZ_ACP_SYSTEM_PROMPT_FILE = $SystemPromptFile
$env:BUZZ_ACP_DISPLAY_NAME = "OpenMontage Agent"
$env:OPENMONTAGE_ROOT = $OpenMontageRoot

. (Join-Path $PSScriptRoot "_agent-owner.ps1")
Set-RoomAgentOwner -BonyRoot $BonyRoot

$env:BUZZ_ACP_AUTO_POST_REPLY = "true"

if ($env:BUZZ_AGENT_PROVIDER -and $agent) {
  $env:BUZZ_ACP_AGENT_COMMAND = $agent
  $env:BUZZ_ACP_AGENT_ARGS = ""
  Write-Host "OpenMontage specialist via buzz-agent provider=$($env:BUZZ_AGENT_PROVIDER)"
} else {
  if (-not $grok) { throw "grok.cmd not found (or set BUZZ_AGENT_PROVIDER for buzz-agent)" }
  $env:BUZZ_ACP_AGENT_COMMAND = $grok
  $env:BUZZ_ACP_AGENT_ARGS = "agent,stdio"
  Write-Host "OpenMontage specialist via Grok CLI + bony-room-tools-mcp"
}

Write-Host "  acp:  $acp"
Write-Host "  mcp:  $mcp"
Write-Host "  OPENMONTAGE_ROOT=$OpenMontageRoot"
& $acp
