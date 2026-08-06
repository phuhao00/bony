# Mesh room bot membership carefully:
# - Do NOT re-join every historical "Local Room" clone
# - Keep a single canonical Local Room (lexicographically first id, or BUZZ_ROOM_CHANNEL_ID)
# - Mesh other distinct human channels normally
param(
  [string]$RelayHttp = "http://localhost:3000",
  [string]$KeysDir = (Join-Path $PSScriptRoot "keys"),
  [string]$CanonicalLocalRoomName = "Local Room"
)
$ErrorActionPreference = "Continue"
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

$buzz = Find-Bin "buzz" @($BonyRoot, $BuzzRoot)
if (-not $buzz) { throw "buzz CLI missing" }

$roster = @()
foreach ($id in @("grok", "zeroclaw", "unity", "openmontage", "docsmith")) {
  $kf = Join-Path $KeysDir "$id.json"
  if (-not (Test-Path $kf)) { continue }
  $k = Get-Content $kf -Raw | ConvertFrom-Json
  $sk = if ($k.nsec) { $k.nsec } elseif ($k.private_key) { $k.private_key } else { $null }
  $pk = $k.public_key_hex
  if (-not $sk -or -not $pk) { continue }
  $roster += [pscustomobject]@{ Id = $id; Secret = $sk; Pubkey = $pk.ToLowerInvariant() }
}
if ($roster.Count -eq 0) { throw "no room agent keys in $KeysDir" }

$env:BUZZ_RELAY_URL = $RelayHttp
$channelIds = New-Object 'System.Collections.Generic.HashSet[string]'
$localRoomIds = New-Object 'System.Collections.Generic.List[string]'

foreach ($r in $roster) {
  $env:BUZZ_PRIVATE_KEY = $r.Secret
  try {
    $raw = & $buzz --format compact channels list --member 2>$null | Out-String
    $listed = $raw | ConvertFrom-Json
    if ($null -eq $listed) { continue }
    if ($listed -isnot [System.Array]) { $listed = @($listed) }
    foreach ($c in $listed) {
      $cid = [string]$c.channel_id
      $nm = [string]$c.name
      if (-not $cid) { continue }
      if ($nm -eq $CanonicalLocalRoomName) {
        if (-not $localRoomIds.Contains($cid)) { [void]$localRoomIds.Add($cid) }
      } elseif ($nm -like 'Local Room retired*') {
        continue
      } else {
        [void]$channelIds.Add($cid)
      }
    }
  } catch {}
}

# Exactly one Local Room for mesh
$canonicalLocal = $null
if ($env:BUZZ_ROOM_CHANNEL_ID -and $localRoomIds.Contains($env:BUZZ_ROOM_CHANNEL_ID)) {
  $canonicalLocal = $env:BUZZ_ROOM_CHANNEL_ID
} elseif ($localRoomIds.Count -gt 0) {
  $canonicalLocal = ($localRoomIds | Sort-Object | Select-Object -First 1)
}
if ($canonicalLocal) {
  [void]$channelIds.Add($canonicalLocal)
  Write-Host ("  canonical Local Room: {0} (of {1} seen)" -f $canonicalLocal, $localRoomIds.Count)
}

Write-Host "==> Bot mesh: $($channelIds.Count) channels x $($roster.Count) agents"
$added = 0
foreach ($ch in $channelIds) {
  $inviter = $null
  foreach ($r in $roster) {
    $env:BUZZ_PRIVATE_KEY = $r.Secret
    try {
      $mraw = & $buzz --format compact channels members --channel $ch 2>$null | Out-String
      $members = $mraw | ConvertFrom-Json
      if ($null -eq $members) { continue }
      if ($members -isnot [System.Array]) { $members = @($members) }
      if ($members.Count -gt 0) { $inviter = $r; break }
    } catch {}
  }
  if (-not $inviter) { continue }
  $env:BUZZ_PRIVATE_KEY = $inviter.Secret
  $memberPks = @()
  try {
    $mraw = & $buzz --format compact channels members --channel $ch 2>$null | Out-String
    $members = $mraw | ConvertFrom-Json
    if ($members -isnot [System.Array]) { $members = @($members) }
    $memberPks = @($members | ForEach-Object { ([string]$_.pubkey).ToLowerInvariant() })
  } catch { continue }

  foreach ($r in $roster) {
    if ($memberPks -contains $r.Pubkey) { continue }
    $env:BUZZ_PRIVATE_KEY = $inviter.Secret
    $out = ""
    try {
      $out = & $buzz channels add-member --channel $ch --pubkey $r.Pubkey --role bot 2>&1 | Out-String
    } catch {
      $out = "$_"
    }
    $ok = ($out -match 'accepted.:true') -or ($out -match '"accepted"\s*:\s*true')
    if (-not $ok) {
      # Try join as self
      $env:BUZZ_PRIVATE_KEY = $r.Secret
      try {
        $out2 = & $buzz channels join --channel $ch 2>&1 | Out-String
        if ($out2 -match 'accepted' -or $LASTEXITCODE -eq 0) { $ok = $true }
      } catch {}
    }
    if ($ok) {
      $added++
      $short = if ($ch.Length -gt 8) { $ch.Substring(0, 8) } else { $ch }
      Write-Host ("  + {0} -> {1}..." -f $r.Id, $short)
    }
  }
}
Write-Host ("  mesh complete (added {0} memberships)" -f $added)
Remove-Item Env:BUZZ_PRIVATE_KEY -ErrorAction SilentlyContinue
