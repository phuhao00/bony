# Start Grok as Buzz room coordinator (subscribe=all, public auto-routing).
param(
  [string]$BuzzRoot = "",
  [string]$BonyRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path,
  [string]$RelayUrl = "ws://localhost:3000",
  [string]$Cwd = $BonyRoot,
  [string]$KeysFile = (Join-Path $PSScriptRoot "keys\grok.json"),
  [string]$SystemPromptFile = (Join-Path $PSScriptRoot "prompts\grok-coordinator.md")
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
  $cmd = Get-Command $name -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  return $null
}

$acp = Find-Bin "buzz-acp" @($BuzzRoot)
$devMcp = Find-Bin "buzz-dev-mcp" @($BuzzRoot)
$grokCmd = Get-Command grok -ErrorAction SilentlyContinue
$grok = if ($grokCmd) { $grokCmd.Source } else { $null }
if (-not $grok) {
  $grokCmdPath = Join-Path $env:APPDATA "npm\grok.cmd"
  if (Test-Path $grokCmdPath) { $grok = $grokCmdPath }
}
if (-not $acp) { throw "buzz-acp not built. Run scripts/buzz-room/build-tools.ps1" }
if (-not $grok) { throw "grok CLI not found on PATH" }
if (-not $devMcp) { Write-Warning "buzz-dev-mcp missing — Grok will not get shell/buzz tools via MCP" }

$nsec = $env:BUZZ_PRIVATE_KEY
if (-not $nsec -and (Test-Path $KeysFile)) {
  $k = Get-Content $KeysFile -Raw | ConvertFrom-Json
  if ($k.nsec) { $nsec = $k.nsec }
  elseif ($k.private_key) { $nsec = $k.private_key }
  elseif ($k.secret_key) { $nsec = $k.secret_key }
}
if (-not $nsec) {
  throw "Set BUZZ_PRIVATE_KEY or put nsec in $KeysFile"
}

$env:BUZZ_RELAY_URL = $RelayUrl
$env:BUZZ_PRIVATE_KEY = $nsec
$env:BUZZ_ACP_AGENT_COMMAND = $grok
$env:BUZZ_ACP_AGENT_ARGS = "agent,stdio"
if ($devMcp) { $env:BUZZ_ACP_MCP_COMMAND = $devMcp }
$env:BUZZ_ACP_SUBSCRIBE = "all"
$env:BUZZ_ACP_RESPOND_TO = "anyone"
$env:BUZZ_ACP_PERMISSION_MODE = "accept-edits"
$env:BUZZ_ACP_SYSTEM_PROMPT_FILE = $SystemPromptFile

Write-Host "Grok coordinator"
Write-Host "  buzz:    $BuzzRoot"
Write-Host "  relay:   $RelayUrl"
Write-Host "  cwd:     $Cwd"
Write-Host "  grok:    $grok"
Write-Host "  acp:     $acp"
Write-Host "  mcp:     $devMcp"
Write-Host "  prompt:  $SystemPromptFile"
Write-Host "  subscribe=all  respond-to=anyone  permission=accept-edits"

Set-Location $Cwd
& $acp
