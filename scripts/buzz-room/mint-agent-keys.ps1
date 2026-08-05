# Mint Nostr keys for room agents (stored under scripts/buzz-room/keys).
param(
  [string]$BuzzRoot = "",
  [string]$KeysDir = (Join-Path $PSScriptRoot "keys")
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
if (-not $BuzzRoot) { $BuzzRoot = Get-BuzzRoot }
New-Item -ItemType Directory -Force -Path $KeysDir | Out-Null

$admin = Join-Path $BuzzRoot "target\debug\buzz-admin.exe"
if (-not (Test-Path $admin)) {
  $admin = Join-Path $BuzzRoot "target\release\buzz-admin.exe"
}
if (-not (Test-Path $admin)) {
  throw "buzz-admin not found. Run scripts/buzz-room/build-tools.ps1 first."
}

$agents = @("grok", "zeroclaw", "unity", "openmontage")
foreach ($name in $agents) {
  $out = Join-Path $KeysDir "$name.json"
  if (Test-Path $out) {
    Write-Host "exists: $out"
    continue
  }
  Write-Host "minting $name ..."
  $raw = & $admin generate-key 2>&1 | Out-String
  if ($LASTEXITCODE -ne 0) {
    throw "buzz-admin generate-key failed: $raw"
  }
  $pub = if ($raw -match "Public key:\s+([0-9a-fA-F]{64})") { $Matches[1] } else { $null }
  $sec = if ($raw -match "Secret key:\s+([0-9a-fA-F]{64})") { $Matches[1] } else { $null }
  if (-not $pub -or -not $sec) {
    throw "Could not parse key output:`n$raw"
  }
  @{
    display_name = $name
    public_key_hex = $pub
    private_key = $sec
    nsec = $sec
    note = "Hex secret key for BUZZ_PRIVATE_KEY (buzz-admin generate-key)."
  } | ConvertTo-Json | Set-Content -Encoding utf8 $out
  Write-Host "wrote $out  pubkey=$pub"
}

Write-Host "Keys directory: $KeysDir"
Write-Host "Add each public_key_hex / npub as a channel member in Buzz Desktop."
