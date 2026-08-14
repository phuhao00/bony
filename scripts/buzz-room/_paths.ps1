# Shared path helpers for Buzz room scripts (dot-source this file).
$script:BonyRoomScriptDir = $PSScriptRoot
$script:BonyRootDefault = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
# Product lives at the repo root (crates/, desktop/). BuzzRoot == BonyRoot.
$script:BuzzRootDefault = $script:BonyRootDefault

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
  return $script:BuzzRootDefault
}
