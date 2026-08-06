# Mesh room bot membership: wherever one room agent is already a member,
# ensure Grok + specialists are also bot members so Grok can auto-route
# un-@ human messages (specialists stay mentions-only for reply).
param(
  [string]$RelayHttp = "http://localhost:3000",
  [string]$KeysDir = (Join-Path $PSScriptRoot "keys")
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
foreach ($id in @("grok", "zeroclaw", "unity", "openmontage")) {
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
$targetPks = @($roster | ForEach-Object { $_.Pubkey })
$channelIds = New-Object 'System.Collections.Generic.HashSet[string]'

foreach ($r in $roster) {
  $env:BUZZ_PRIVATE_KEY = $r.Secret
  try {
    $raw = & $buzz --format compact channels list 2>&1 | Out-String
    $listed = $raw | ConvertFrom-Json
    if ($null -eq $listed) { continue }
    if ($listed -isnot [System.Array]) { $listed = @($listed) }
    foreach ($c in $listed) {
      if ($c.channel_id) { [void]$channelIds.Add([string]$c.channel_id) }
    }
  } catch {}
}

Write-Host "==> Bot mesh: $($channelIds.Count) channels x $($roster.Count) agents"
$added = 0
foreach ($ch in $channelIds) {
  # Prefer an inviter already in the channel (any roster member who can list members).
  $inviter = $null
  foreach ($r in $roster) {
    $env:BUZZ_PRIVATE_KEY = $r.Secret
    try {
      $mraw = & $buzz --format compact channels members --channel $ch 2>&1 | Out-String
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
    $mraw = & $buzz --format compact channels members --channel $ch 2>&1 | Out-String
    $members = $mraw | ConvertFrom-Json
    if ($members -isnot [System.Array]) { $members = @($members) }
    $memberPks = @($members | ForEach-Object { ([string]$_.pubkey).ToLowerInvariant() })
  } catch { continue }

  foreach ($r in $roster) {
    if ($memberPks -contains $r.Pubkey) { continue }
    $ok = $false
    if ($out -match 'accepted.:true') { $ok = $true }
    if ($out -match '"accepted":\s*true') { $ok = $true }
    if ($ok) {
      $added++
      $short = $ch
      if ($ch.Length -gt 8) { $short = $ch.Substring(0, 8) }
      Write-Host ("  + {0} -> {1}..." -f $r.Id, $short)
    }
  }
}
Write-Host ("  mesh complete (added {0} memberships)" -f $added)
Remove-Item Env:BUZZ_PRIVATE_KEY -ErrorAction SilentlyContinue
