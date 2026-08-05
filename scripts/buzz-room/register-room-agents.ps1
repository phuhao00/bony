# Register room agents so Desktop can see them:
# 1) kind:0 name + kind:10100 directory on the local relay
# 2) open "Local Room" channel with all four as bot members
# 3) upsert managed-agents.json entries (Agents page cards)
param(
  [string]$RelayHttp = "http://localhost:3000",
  [string]$RelayWs = "ws://localhost:3000",
  [string]$KeysDir = (Join-Path $PSScriptRoot "keys"),
  [string]$ChannelName = "Local Room",
  [switch]$SkipManagedAgents,
  [switch]$SkipChannel
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
$BonyRoot = Get-BonyRoot
$BuzzRoot = Get-BuzzRoot

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

$buzz = Find-Bin "buzz" @($BonyRoot, $BuzzRoot)
$acp = Find-Bin "buzz-acp" @($BonyRoot, $BuzzRoot)
$devMcp = Find-Bin "buzz-dev-mcp" @($BonyRoot, $BuzzRoot)
$roomMcp = Find-Bin "bony-room-tools-mcp" @($BonyRoot, $BuzzRoot)
$zeroclaw = "$env:USERPROFILE\.bony-build\zeroclaw\target\release\zeroclaw.exe"
$grok = Find-Grok
if (-not $buzz) { throw "buzz CLI missing — cargo build -p buzz" }

$agents = @(
  @{
    Id = "grok"; DisplayName = "Grok"; About = "Room coordinator"; Keys = "grok.json"
    AgentCommand = if ($grok) { $grok } else { "grok.cmd" }
    AgentArgs = @("agent", "stdio"); Mcp = $devMcp; RespondTo = "anyone"
  },
  @{
    Id = "zeroclaw"; DisplayName = "ZeroClaw"; About = "ZeroClaw specialist"; Keys = "zeroclaw.json"
    AgentCommand = $zeroclaw; AgentArgs = @("acp"); Mcp = ""; RespondTo = "anyone"
  },
  @{
    Id = "unity"; DisplayName = "Unity"; About = "Unity specialist"; Keys = "unity.json"
    AgentCommand = if ($grok) { $grok } else { "grok.cmd" }
    AgentArgs = @("agent", "stdio"); Mcp = $roomMcp; RespondTo = "anyone"
  },
  @{
    Id = "openmontage"; DisplayName = "OpenMontage"; About = "OpenMontage specialist"; Keys = "openmontage.json"
    AgentCommand = if ($grok) { $grok } else { "grok.cmd" }
    AgentArgs = @("agent", "stdio"); Mcp = $roomMcp; RespondTo = "anyone"
  }
)

# Map id -> resolved key material
$roster = @()
foreach ($a in $agents) {
  $kf = Join-Path $KeysDir $a.Keys
  if (-not (Test-Path $kf)) { throw "missing keys: $kf (run mint-agent-keys.ps1)" }
  $k = Get-Content $kf -Raw | ConvertFrom-Json
  $sk = if ($k.nsec) { $k.nsec } elseif ($k.private_key) { $k.private_key } else { $null }
  if (-not $sk) { throw "no private key in $kf" }
  $pk = $k.public_key_hex
  if (-not $pk) { throw "no public_key_hex in $kf" }
  $roster += [pscustomobject]@{
    Id = $a.Id; DisplayName = $a.DisplayName; About = $a.About
    Secret = $sk; Pubkey = $pk
    AgentCommand = $a.AgentCommand; AgentArgs = $a.AgentArgs
    Mcp = [string]$a.Mcp; RespondTo = $a.RespondTo
  }
}

$env:BUZZ_RELAY_URL = $RelayHttp

Write-Host "==> Directory profiles on $RelayHttp"
foreach ($r in $roster) {
  $env:BUZZ_PRIVATE_KEY = $r.Secret
  $env:BUZZ_ACP_DISPLAY_NAME = $r.DisplayName
  $env:BUZZ_ACP_RESPOND_TO = $r.RespondTo
  Write-Host "  $($r.DisplayName) ($($r.Pubkey.Substring(0,12))...)"
  & $buzz users set-profile --name $r.DisplayName --about $r.About | Out-Host
  if ($LASTEXITCODE -ne 0) { throw "set-profile failed for $($r.Id)" }
  & $buzz channels set-add-policy --policy anyone | Out-Host
  if ($LASTEXITCODE -ne 0) { throw "set-add-policy failed for $($r.Id)" }
  & $buzz users set-presence --status online | Out-Host
}

$channelId = $null
if (-not $SkipChannel) {
  Write-Host "==> Channel '$ChannelName'"
  $grok = $roster | Where-Object Id -eq "grok" | Select-Object -First 1
  $env:BUZZ_PRIVATE_KEY = $grok.Secret
  # Prefer create; if name already exists relay may error — fall back to listing briefly.
  $createdRaw = & $buzz channels create --name $ChannelName --type stream --visibility open --description "Local room stack agents" 2>&1 | Out-String
  try {
    $created = $createdRaw | ConvertFrom-Json
    $channelId = $created.channel_id
  } catch { $channelId = $null }
  if (-not $channelId) {
    $listRaw = & $buzz --format compact channels list 2>&1 | Out-String
    try {
      $listed = $listRaw | ConvertFrom-Json
      if ($listed -is [System.Array]) {
        $hit = $listed | Where-Object { $_.name -eq $ChannelName } | Select-Object -First 1
        if ($hit) { $channelId = $hit.channel_id }
      } elseif ($listed.name -eq $ChannelName) {
        $channelId = $listed.channel_id
      }
    } catch {}
  }
  if (-not $channelId) {
    throw "could not create or find channel '$ChannelName': $createdRaw"
  }
  Write-Host "  channel $channelId"

  foreach ($r in $roster) {
    if ($r.Id -eq "grok") { continue }
    $env:BUZZ_PRIVATE_KEY = $grok.Secret
    & $buzz channels add-member --channel $channelId --pubkey $r.Pubkey --role bot 2>&1 | Out-Host
  }
  # Ensure each agent is a member even if add-member failed (re-join)
  foreach ($r in $roster) {
    $env:BUZZ_PRIVATE_KEY = $r.Secret
    & $buzz channels join --channel $channelId 2>&1 | Out-Null
  }
  Write-Host "  members ready in channel $channelId"
}

if (-not $SkipManagedAgents) {
  $storeDir = Join-Path $env:APPDATA "xyz.block.buzz.app\agents"
  $storePath = Join-Path $storeDir "managed-agents.json"
  New-Item -ItemType Directory -Force -Path $storeDir | Out-Null
  $payload = @{
    storePath   = $storePath
    bonesRoot   = $BonyRoot
    acp         = $(if ($acp) { $acp } else { "buzz-acp" })
    relayWs     = $RelayWs
    agents      = @($roster | ForEach-Object {
      @{
        name           = $_.DisplayName
        about          = $_.About
        pubkey         = $_.Pubkey
        secret         = $_.Secret
        agent_command  = $_.AgentCommand
        agent_args     = @($_.AgentArgs)
        mcp_command    = $_.Mcp
      }
    })
  }
  $payloadJson = $payload | ConvertTo-Json -Depth 6 -Compress
  $py = @'
import json, sys
from datetime import datetime, timezone
from pathlib import Path

data = json.loads(sys.stdin.read())
store = Path(data["storePath"])
store.parent.mkdir(parents=True, exist_ok=True)
existing = []
if store.exists():
    try:
        raw = json.loads(store.read_text(encoding="utf-8-sig"))
        if isinstance(raw, list):
            existing = [a for a in raw if isinstance(a, dict) and a.get("name") and a.get("pubkey")]
    except Exception:
        existing = []
names = {a["name"] for a in data["agents"]}
pks = {(a["pubkey"] or "").lower() for a in data["agents"]}
kept = [a for a in existing if (a.get("pubkey") or "").lower() not in pks and a.get("name") not in names]
now = datetime.now(timezone.utc).isoformat()
acp = data["acp"]
for a in data["agents"]:
    kept.append({
        "pubkey": a["pubkey"],
        "name": a["name"],
        "private_key_nsec": a["secret"],
        "relay_url": data["relayWs"],
        "acp_command": acp,
        "agent_command": a["agent_command"],
        "agent_args": a["agent_args"],
        "mcp_command": a.get("mcp_command") or "",
        "system_prompt": f"Local room agent: {a['name']}. {a.get('about') or ''}".strip(),
        "respond_to": "anyone",
        "respond_to_allowlist": [],
        "backend": {"type": "local"},
        "is_active": True,
        "is_builtin": False,
        "start_on_app_launch": False,
        "auto_restart_on_config_change": False,
        "parallelism": 1,
        "turn_timeout_seconds": 320,
        "created_at": now,
        "updated_at": now,
        "last_error": None,
        "last_error_code": None,
        "last_exit_code": None,
        "runtime_pid": None,
        "auth_tag": None,
        "avatar_url": None,
        "persona_id": None,
        "team_id": None,
    })
    print(f"  managed-agents += {a['name']}")
store.write_text(json.dumps(kept, indent=2), encoding="utf-8")
print(f"==> wrote {store} ({len(kept)} agents total)")
print("    Restart Buzz Desktop (or re-open Agents) to refresh the list.")
'@
  $payloadJson | python -c $py
  if ($LASTEXITCODE -ne 0) { throw "managed-agents upsert failed" }
}

Write-Host ""
Write-Host "Room agents registered."
Write-Host "  Relay:   $RelayHttp / $RelayWs"
if ($channelId) { Write-Host "  Channel: $ChannelName ($channelId)" }
Write-Host "  Next: join community ws://localhost:3000 and open '$ChannelName'."
Write-Host "  @Grok works with subscribe=all; specialists respond on @mention."
