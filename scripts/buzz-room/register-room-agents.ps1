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
  [switch]$SkipChannel,
  # Replace managed-agents.json with only the four room agents (drops ghosts /
  # blank pubkey stubs that break Start agent). Default: on for local stack.
  [switch]$ReplaceStore = $true
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
  # Desktop identifier may be xyz.block.buzz.app (fast path) or .dev (tauri override).
  $storePaths = @(
    (Join-Path $env:APPDATA "xyz.block.buzz.app\agents\managed-agents.json"),
    (Join-Path $env:APPDATA "xyz.block.buzz.app.dev\agents\managed-agents.json")
  )
  $acpCmd = if ($acp) { $acp } else { "buzz-acp" }
  $now = (Get-Date).ToUniversalTime().ToString("o")
  $roomRecords = @()
  foreach ($r in $roster) {
    $roomRecords += [ordered]@{
      pubkey                         = $r.Pubkey
      name                           = $r.DisplayName
      private_key_nsec               = $r.Secret
      relay_url                      = $RelayWs
      acp_command                    = $acpCmd
      agent_command                  = $r.AgentCommand
      agent_args                     = @($r.AgentArgs)
      mcp_command                    = $(if ($r.Mcp) { [string]$r.Mcp } else { "" })
      system_prompt                  = ("Local room agent: {0}. {1}" -f $r.DisplayName, $r.About).Trim()
      respond_to                     = "anyone"
      respond_to_allowlist           = @()
      backend                        = @{ type = "local" }
      is_active                      = $true
      is_builtin                     = $false
      # Must stay definition-less. Setting persona_id without a matching
      # definitions[] row makes Desktop resolve OrphanedInstance and refuse
      # mentions with "This agent's configuration is missing".
      # External room harness runs agents via start-external-room-agents.ps1.
      # Desktop auto-spawn would only open setup-listener / dual identity noise.
      start_on_app_launch            = $false
      auto_restart_on_config_change  = $false
      parallelism                    = 1
      turn_timeout_seconds           = 320
      created_at                     = $now
      updated_at                     = $now
      last_error                     = $null
      last_error_code                = $null
      last_exit_code                 = $null
      runtime_pid                    = $null
      auth_tag                       = $null
      avatar_url                     = $null
      team_id                        = $null
    }
    Write-Host ("  managed-agents += {0} ({1}…)" -f $r.DisplayName, $r.Pubkey.Substring(0, [Math]::Min(12, $r.Pubkey.Length)))
  }
  $roomNames = @($roomRecords | ForEach-Object { $_.name })
  $roomPks = @($roomRecords | ForEach-Object { ([string]$_.pubkey).ToLowerInvariant() })

  foreach ($storePath in $storePaths) {
    $dir = Split-Path $storePath -Parent
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $kept = @()
    if ((Test-Path $storePath) -and -not $ReplaceStore) {
      try {
        $raw = Get-Content $storePath -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($raw -is [System.Array]) {
          foreach ($a in $raw) {
            $pk = [string]$a.pubkey
            $nm = [string]$a.name
            if (-not $pk.Trim() -or -not $nm.Trim()) { continue }
            if ($roomPks -contains $pk.ToLowerInvariant()) { continue }
            if ($roomNames -contains $nm) { continue }
            $kept += $a
          }
        }
      } catch {}
    } elseif (Test-Path $storePath) {
      Write-Host "  replace mode: dropping previous $storePath"
    }
    $final = @($kept) + @($roomRecords)
    $json = $final | ConvertTo-Json -Depth 8
    [System.IO.File]::WriteAllText($storePath, $json, [System.Text.UTF8Encoding]::new($false))
    Write-Host "==> wrote $storePath ($($final.Count) agents)"
  }
  Write-Host "    Restart Buzz Desktop (or re-open Agents) to refresh the list."
}

Write-Host ""
Write-Host "Room agents registered."
Write-Host "  Relay:   $RelayHttp / $RelayWs"
if ($channelId) { Write-Host "  Channel: $ChannelName ($channelId)" }
Write-Host "  Next: join community ws://localhost:3000 and open '$ChannelName'."
Write-Host "  @Grok works with subscribe=all; specialists respond on @mention."
# Do not leak the last agent secret into parent shells / Desktop ProcessStartInfo.
Remove-Item Env:BUZZ_PRIVATE_KEY -ErrorAction SilentlyContinue
Remove-Item Env:NOSTR_PRIVATE_KEY -ErrorAction SilentlyContinue
$env:BUZZ_ACP_DISPLAY_NAME = $null
$env:BUZZ_ACP_RESPOND_TO = $null
