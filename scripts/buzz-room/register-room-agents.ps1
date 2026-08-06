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
$docsMcp = Find-Bin "bony-docs-tools-mcp" @($BonyRoot, $BuzzRoot)
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
  },
  @{
    Id = "docsmith"; DisplayName = "DocSmith"; About = "Docs specialist (PDF/Word/Excel/PPT)"; Keys = "docsmith.json"
    AgentCommand = if ($grok) { $grok } else { "grok.cmd" }
    AgentArgs = @("agent", "stdio"); Mcp = $docsMcp; RespondTo = "anyone"
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
  $prevEap = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  Write-Host "==> Channel '$ChannelName' (single instance; retire extras)"
  $grok = $roster | Where-Object Id -eq "grok" | Select-Object -First 1
  $env:BUZZ_PRIVATE_KEY = $grok.Secret

  function Invoke-BuzzQuiet {
    param([Parameter(ValueFromRemainingArguments = $true)][string[]]$BuzzArgs)
    try {
      & $script:buzz @BuzzArgs 1>$null 2>$null
    } catch {}
  }

  function Get-ChannelList {
    try {
      $listRaw = & $script:buzz --format compact channels list 2>$null | Out-String
      $listed = $listRaw | ConvertFrom-Json
      if ($null -eq $listed) { return @() }
      if ($listed -isnot [System.Array]) { return @($listed) }
      return @($listed)
    } catch {
      return @()
    }
  }

  $all = Get-ChannelList
  $sameName = @($all | Where-Object { [string]$_.name -eq $ChannelName })
  if ($sameName.Count -gt 0) {
    $keep = $null
    if ($env:BUZZ_ROOM_CHANNEL_ID) {
      $keep = $sameName | Where-Object { [string]$_.channel_id -eq $env:BUZZ_ROOM_CHANNEL_ID } | Select-Object -First 1
    }
    if (-not $keep) {
      $keep = $sameName | Sort-Object { [string]$_.channel_id } | Select-Object -First 1
    }
    $channelId = [string]$keep.channel_id
    Write-Host "  reusing $channelId (matched $($sameName.Count) named '$ChannelName')"
    $extras = @($sameName | Where-Object { [string]$_.channel_id -ne $channelId })
    foreach ($ex in $extras) {
      $eid = [string]$ex.channel_id
      $short = if ($eid.Length -gt 8) { $eid.Substring(0, 8) } else { $eid }
      Write-Host "  retire duplicate Local Room $short"
      $env:BUZZ_PRIVATE_KEY = $grok.Secret
      # Archived twins reject leave/update — unarchive first.
      Invoke-BuzzQuiet channels unarchive --channel $eid
      $retiredName = "Local Room retired $short"
      Invoke-BuzzQuiet channels update --channel $eid --name $retiredName
      # Specialists drop membership; Grok remains last owner so channel can stay archived.
      foreach ($r in $roster) {
        if ($r.Id -eq "grok") { continue }
        $env:BUZZ_PRIVATE_KEY = $r.Secret
        Invoke-BuzzQuiet channels leave --channel $eid
      }
      $env:BUZZ_PRIVATE_KEY = $grok.Secret
      Invoke-BuzzQuiet channels archive --channel $eid
    }
  } else {
    Write-Host "  creating new '$ChannelName' (none found)"
    $createdRaw = ""
    try {
      $createdRaw = & $buzz channels create --name $ChannelName --type stream --visibility open --description "Local room stack agents" 2>$null | Out-String
      $created = $createdRaw | ConvertFrom-Json
      $channelId = $created.channel_id
    } catch { $channelId = $null }
    if (-not $channelId) {
      throw "could not create channel '$ChannelName': $createdRaw"
    }
  }
  if (-not $channelId) {
    throw "could not resolve channel '$ChannelName'"
  }
  Write-Host "  channel $channelId"

  $env:BUZZ_PRIVATE_KEY = $grok.Secret
  foreach ($r in $roster) {
    if ($r.Id -eq "grok") { continue }
    Invoke-BuzzQuiet channels add-member --channel $channelId --pubkey $r.Pubkey --role bot
  }
  foreach ($r in $roster) {
    $env:BUZZ_PRIVATE_KEY = $r.Secret
    Invoke-BuzzQuiet channels join --channel $channelId
  }
  Write-Host "  members ready in channel $channelId"
  $ErrorActionPreference = $prevEap
}

if (-not $SkipManagedAgents) {
  # Desktop identifier may be xyz.block.buzz.app (fast path) or .dev (tauri override).
  $storePaths = @(
    (Join-Path $env:APPDATA "xyz.block.buzz.app\agents\managed-agents.json"),
    (Join-Path $env:APPDATA "xyz.block.buzz.app.dev\agents\managed-agents.json")
  )
  # Bony Desktop instance (start-desktop sets BUZZ_DESKTOP_INSTANCE_ID=bony-local)
  $inst = [Environment]::GetEnvironmentVariable("BUZZ_DESKTOP_INSTANCE_ID", "Process")
  if ([string]::IsNullOrWhiteSpace($inst)) { $inst = "bony-local" }
  $storePaths += (Join-Path $env:APPDATA "xyz.block.buzz.app.$inst\agents\managed-agents.json")
  $storePaths = @($storePaths | Select-Object -Unique)

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
      persona_id                     = $null
    }
    Write-Host ("  managed-agents += {0} ({1}…)" -f $r.DisplayName, $r.Pubkey.Substring(0, [Math]::Min(12, $r.Pubkey.Length)))
  }
  $roomNames = @($roomRecords | ForEach-Object { $_.name.ToLowerInvariant() })
  $roomPks = @($roomRecords | ForEach-Object { ([string]$_.pubkey).ToLowerInvariant() })

  function Score-ManagedAgent($a) {
    $s = 0
    $pk = [string]$a.pubkey
    if ($pk.Length -eq 64) { $s += 8 }
    if ($a.private_key_nsec) { $s += 4 }
    if ($a.persona_id) { $s += 2 }
    if ($a.is_builtin) { $s += 1 }
    if ($a.is_active) { $s += 1 }
    return $s
  }

  function Dedupe-ManagedAgents([object[]]$items) {
    $best = @{}
    foreach ($a in $items) {
      if ($null -eq $a) { continue }
      $nm = [string]$a.name
      $pk = [string]$a.pubkey
      if ([string]::IsNullOrWhiteSpace($nm)) { continue }
      # Ghost stubs with empty pubkey break @mentions / start agent.
      if ([string]::IsNullOrWhiteSpace($pk)) { continue }
      $key = $nm.ToLowerInvariant()
      if (-not $best.ContainsKey($key)) {
        $best[$key] = $a
        continue
      }
      if ((Score-ManagedAgent $a) -ge (Score-ManagedAgent $best[$key])) {
        $best[$key] = $a
      }
    }
    # Secondary: same pubkey under two names → keep higher score name only
    $byPk = @{}
    foreach ($kv in $best.GetEnumerator()) {
      $a = $kv.Value
      $pk = ([string]$a.pubkey).ToLowerInvariant()
      if (-not $byPk.ContainsKey($pk)) {
        $byPk[$pk] = $a
        continue
      }
      if ((Score-ManagedAgent $a) -ge (Score-ManagedAgent $byPk[$pk])) {
        $byPk[$pk] = $a
      }
    }
    return @($byPk.Values)
  }

  foreach ($storePath in $storePaths) {
    $dir = Split-Path $storePath -Parent
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $kept = @()
    if (Test-Path $storePath) {
      try {
        $raw = Get-Content $storePath -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($raw -is [System.Array]) {
          foreach ($a in $raw) {
            $pk = [string]$a.pubkey
            $nm = [string]$a.name
            if ([string]::IsNullOrWhiteSpace($nm)) { continue }
            if ([string]::IsNullOrWhiteSpace($pk) -or $pk.Length -ne 64) { continue }
            $nmL = $nm.ToLowerInvariant()
            $pkL = $pk.ToLowerInvariant()
            # Room seats always rewritten from keys — skip old copies.
            if ($roomPks -contains $pkL) { continue }
            if ($roomNames -contains $nmL) { continue }
            if ($ReplaceStore) {
              # Keep only real builtins (not orphan room clones).
              $isBu = ($a.is_builtin -eq $true) -or ([string]$a.persona_id -match '^builtin:')
              if (-not $isBu) { continue }
            }
            # Base64 data-URL avatars (~200KB) break Windows keyring (2560-byte
            # CREDENTIAL blob) when Desktop flushes secrets — strip them.
            try {
              $av = [string]$a.avatar_url
              if ($av.Length -gt 2000) { $a.avatar_url = $null }
            } catch {}
            # Never let Desktop spawn room external seats (double process).
            try { $a.start_on_app_launch = $false } catch {}
            try { $a.auto_restart_on_config_change = $false } catch {}
            try { $a.runtime_pid = $null } catch {}
            $kept += $a
          }
        }
      } catch {
        Write-Warning "  could not parse $storePath — rewriting: $_"
      }
    }
    $final = Dedupe-ManagedAgents (@($kept) + @($roomRecords))
    # Force room seats never auto-spawn from Desktop.
    $final = @($final | ForEach-Object {
      $nmL = ([string]$_.name).ToLowerInvariant()
      $pkL = ([string]$_.pubkey).ToLowerInvariant()
      if (($roomNames -contains $nmL) -or ($roomPks -contains $pkL)) {
        try { $_.start_on_app_launch = $false } catch {}
        try { $_.auto_restart_on_config_change = $false } catch {}
        try { $_.runtime_pid = $null } catch {}
      }
      $_
    })
    $json = $final | ConvertTo-Json -Depth 8
    [System.IO.File]::WriteAllText($storePath, $json, [System.Text.UTF8Encoding]::new($false))
    Write-Host "==> wrote $storePath ($($final.Count) unique agents)"
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
