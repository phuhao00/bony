# Shrink Buzz cargo target without wiping executables (room sidecars / desktop).
# Typical win: drop multi-GB of *.pdb + incremental; leave deps rlib for faster rebuilds.
param(
  [string]$BuzzRoot = "",
  [switch]$Deep,
  [switch]$Nuclear
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
if (-not $BuzzRoot) { $BuzzRoot = Get-BuzzRoot }
$shared = Join-Path $BuzzRoot "target"
$legacy = Join-Path $BuzzRoot "desktop\src-tauri\target"

function Get-DirGb([string]$p) {
  if (-not (Test-Path $p)) { return 0 }
  $sum = (Get-ChildItem $p -Recurse -File -EA SilentlyContinue | Measure-Object Length -Sum).Sum
  if (-not $sum) { return 0 }
  return [math]::Round($sum / 1GB, 2)
}

Write-Host ("==> Before: shared={0} GB  legacy={1} GB" -f (Get-DirGb $shared), (Get-DirGb $legacy))

if (Test-Path $legacy) {
  Write-Host "    remove duplicate desktop/src-tauri/target"
  Remove-Item -LiteralPath $legacy -Recurse -Force -ErrorAction SilentlyContinue
}

if (-not (Test-Path $shared)) {
  Write-Host "    no target dir"
  exit 0
}

if ($Nuclear) {
  Write-Host "    NUCLEAR: wipe target (next cargo is full recompile)"
  Remove-Item -LiteralPath $shared -Recurse -Force
  Write-Host "    After: 0 GB"
  exit 0
}

$debug = Join-Path $shared "debug"
if (Test-Path $debug) {
  Write-Host "    delete PDBs..."
  Get-ChildItem $debug -Recurse -Filter *.pdb -File -EA SilentlyContinue | Remove-Item -Force -EA SilentlyContinue
  $inc = Join-Path $debug "incremental"
  if (Test-Path $inc) {
    Write-Host "    delete debug/incremental"
    Remove-Item -LiteralPath $inc -Recurse -Force -EA SilentlyContinue
  }
  if ($Deep) {
    $build = Join-Path $debug "build"
    if (Test-Path $build) {
      Write-Host "    delete debug/build"
      Remove-Item -LiteralPath $build -Recurse -Force -EA SilentlyContinue
    }
  }
}

$rel = Join-Path $shared "release"
if (Test-Path $rel) {
  Get-ChildItem $rel -Recurse -Filter *.pdb -File -EA SilentlyContinue | Remove-Item -Force -EA SilentlyContinue
}

Write-Host ("==> After:  shared={0} GB" -f (Get-DirGb $shared))
Write-Host "    Keeps rlibs/exes. Use -Nuclear only if you accept full recompile."
