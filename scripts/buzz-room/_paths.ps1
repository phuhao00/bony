# Shared path helpers for Buzz room scripts (dot-source this file).
$script:BonyRoomScriptDir = $PSScriptRoot
$script:BonyRootDefault = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$script:BuzzRootDefault = Join-Path $script:BonyRootDefault "third_party\buzz"
$script:BuzzPatchesDir = Join-Path $script:BonyRootDefault "integrations\buzz\patches"

function Get-BonyRoot {
  param([string]$Override)
  if ($Override) { return (Resolve-Path $Override).Path }
  return $script:BonyRootDefault
}

function Get-BuzzRoot {
  param([string]$Override)
  if ($Override) {
    if (-not (Test-Path $Override)) { throw "BuzzRoot not found: $Override" }
    return (Resolve-Path $Override).Path
  }
  if (Test-Path $script:BuzzRootDefault) {
    return (Resolve-Path $script:BuzzRootDefault).Path
  }
  throw @"
Buzz checkout missing at $($script:BuzzRootDefault).
Run: powershell -ExecutionPolicy Bypass -File scripts/buzz-room/setup-buzz.ps1
"@
}
