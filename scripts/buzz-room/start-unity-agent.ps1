# Start Unity specialist: buzz-agent + bony-room-tools-mcp (mentions-only).
param(
  [string]$BuzzRoot = "",
  [string]$BonyRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path,
  [string]$RelayUrl = "ws://localhost:3000",
  [string]$KeysFile = (Join-Path $PSScriptRoot "keys\unity.json"),
  [string]$SystemPromptFile = (Join-Path $PSScriptRoot "prompts\unity-specialist.md")
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
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
$acp = Find-Bin "buzz-acp" @($BuzzRoot)
$agent = Find-Bin "buzz-agent" @($BuzzRoot)
$mcp = Find-Bin "bony-room-tools-mcp" @($BonyRoot)
if (-not $acp) { throw "buzz-acp missing" }
if (-not $agent) { throw "buzz-agent missing" }
if (-not $mcp) { throw "bony-room-tools-mcp missing — run build-tools.ps1" }

$nsec = $env:BUZZ_PRIVATE_KEY
if (-not $nsec -and (Test-Path $KeysFile)) {
  $k = Get-Content $KeysFile -Raw | ConvertFrom-Json
  if ($k.nsec) { $nsec = $k.nsec }
  elseif ($k.private_key) { $nsec = $k.private_key }
}
if (-not $nsec) { throw "need nsec in env or $KeysFile" }

$env:BUZZ_RELAY_URL = $RelayUrl
$env:BUZZ_PRIVATE_KEY = $nsec
$env:BUZZ_ACP_AGENT_COMMAND = $agent
$env:BUZZ_ACP_AGENT_ARGS = ""
$env:BUZZ_ACP_MCP_COMMAND = $mcp
$env:BUZZ_ACP_SUBSCRIBE = "mentions"
$env:BUZZ_ACP_RESPOND_TO = "anyone"
$env:BUZZ_ACP_PERMISSION_MODE = "accept-edits"
$env:BUZZ_ACP_SYSTEM_PROMPT_FILE = $SystemPromptFile

Write-Host "Unity specialist buzz-agent + $mcp"
Write-Host "  buzz: $BuzzRoot"
& $acp
