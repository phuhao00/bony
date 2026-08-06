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

$BonyRoot = Get-BonyRoot
$acp = Find-Bin "buzz-acp" @($BonyRoot, $BuzzRoot)
$devMcp = Find-Bin "buzz-dev-mcp" @($BonyRoot, $BuzzRoot)
# Prefer Windows npm shims (.cmd) — Get-Command often returns grok.ps1 which
# CreateProcess cannot launch as the agent binary.
$grok = $null
foreach ($candidate in @(
    (Join-Path $env:APPDATA "npm\grok.cmd"),
    (Join-Path $env:LOCALAPPDATA "npm\grok.cmd"),
    "grok.cmd"
  )) {
  if ($candidate -eq "grok.cmd") {
    $cmd = Get-Command grok.cmd -ErrorAction SilentlyContinue
    if ($cmd) { $grok = $cmd.Source; break }
  } elseif (Test-Path $candidate) {
    $grok = $candidate
    break
  }
}
if (-not $grok) {
  $grokCmd = Get-Command grok -ErrorAction SilentlyContinue
  if ($grokCmd -and $grokCmd.Source -notmatch '\.ps1$') {
    $grok = $grokCmd.Source
  }
}
if (-not $acp) { throw "buzz-acp not built. Run scripts/buzz-room/build-tools.ps1" }
if (-not $grok) { throw "grok CLI not found (expected grok.cmd on PATH or under %APPDATA%\npm)" }
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
# Critical: bare subscribe=all with empty kinds = wildcard → reactions, presence,
# control noise all become turns → multi "Understood…" spam. Restrict to stream
# message kinds only (same set Mentions mode uses by default).
$env:BUZZ_ACP_KINDS = "9,46010,40007"
$env:BUZZ_ACP_RESPOND_TO = "anyone"
$env:BUZZ_ACP_PERMISSION_MODE = "accept-edits"
$env:BUZZ_ACP_SYSTEM_PROMPT_FILE = $SystemPromptFile
$env:BUZZ_ACP_DISPLAY_NAME = "Grok"
# Fast path: don't cancel in-flight turns when messages pile up (re-asks, specialist posts).
$env:BUZZ_ACP_MULTIPLE_EVENT_HANDLING = "queue"
$env:BUZZ_ACP_CONTEXT_MESSAGE_LIMIT = "6"
$env:BUZZ_ACP_NO_MEMORY = "true"

. (Join-Path $PSScriptRoot "_agent-owner.ps1")
Set-RoomAgentOwner -BonyRoot (Get-BonyRoot)

# Grok is subscribe=all and must post visible kind:9 replies (stream alone is UI-invisible).
# Specialists often don't call buzz messages send either — same auto-post safety net.
$env:BUZZ_ACP_AUTO_POST_REPLY = "true"
# Mid-turn coding status posts (tool starts) — default on with AUTO_POST; can set 0 to disable.
$env:BUZZ_ACP_PROGRESS_POST = "true"

# Auto-post "@ZeroClaw …" must include p-tags or Mentions-mode specialists never fire.
. (Join-Path $PSScriptRoot "_mention-map.ps1")
Set-RoomAgentMentionMap

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
