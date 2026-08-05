# Initialize and maintain the Block/Buzz git submodule under third_party/buzz.
param(
  [string]$BonyRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path,
  [switch]$ForceReapplyPatches,
  [switch]$SkipPatches,
  [switch]$Update  # pull submodule to remote tip (optional)
)
$ErrorActionPreference = "Stop"
Set-Location $BonyRoot

$BuzzRoot = Join-Path $BonyRoot "third_party\buzz"
$PatchesDir = Join-Path $BonyRoot "integrations\buzz\patches"

function Test-BuzzTree([string]$Path) {
  return (Test-Path (Join-Path $Path "Cargo.toml")) -and (Test-Path (Join-Path $Path "crates\buzz-acp"))
}

if (-not (Test-Path (Join-Path $BuzzRoot ".git")) -and -not (Test-BuzzTree $BuzzRoot)) {
  Write-Host "==> git submodule update --init third_party/buzz"
  git submodule update --init --recursive third_party/buzz
  if ($LASTEXITCODE -ne 0) { throw "submodule update failed" }
} elseif (-not (Test-BuzzTree $BuzzRoot)) {
  Write-Host "==> git submodule update --init third_party/buzz"
  git submodule update --init --recursive third_party/buzz
  if ($LASTEXITCODE -ne 0) { throw "submodule update failed" }
} else {
  Write-Host "==> Submodule present at $BuzzRoot"
}

if ($Update) {
  Write-Host "==> Updating submodule to origin tip"
  git submodule update --remote third_party/buzz
  if ($LASTEXITCODE -ne 0) { throw "submodule remote update failed" }
}

if (-not (Test-BuzzTree $BuzzRoot)) {
  throw "Buzz tree invalid: $BuzzRoot"
}

if ($SkipPatches) {
  Write-Host "Skip patches"
  Write-Host "BuzzRoot=$BuzzRoot"
  exit 0
}

$cfg = Join-Path $BuzzRoot "crates\buzz-acp\src\config.rs"
$alreadyHasGrok = (Test-Path $cfg) -and (Select-String -Path $cfg -Pattern '"grok" \| "xai-grok"' -Quiet)

if ($alreadyHasGrok -and -not $ForceReapplyPatches) {
  Write-Host "==> Grok runtime already present on phuhao00/buzz pin (skip apply)"
} elseif (Test-Path $PatchesDir) {
  Write-Host "==> Applying bony Grok patches (local worktree; does not change pinned submodule commit until you commit inside submodule)"
  Push-Location $BuzzRoot
  try {
    foreach ($pf in (Get-ChildItem $PatchesDir -Filter "*.patch" | Sort-Object Name)) {
      Write-Host "    $($pf.Name)"
      git apply --check $pf.FullName 2>$null
      if ($LASTEXITCODE -eq 0) {
        git apply $pf.FullName
        if ($LASTEXITCODE -ne 0) { throw "git apply failed: $($pf.Name)" }
      } else {
        git apply --reverse --check $pf.FullName 2>$null
        if ($LASTEXITCODE -eq 0) {
          Write-Host "    already applied"
        } else {
          Write-Warning "Could not apply $($pf.Name) cleanly"
        }
      }
    }
  } finally {
    Pop-Location
  }
}

Write-Host ""
Write-Host "Buzz submodule: $BuzzRoot"
Write-Host "Pinned commit:  $(git -C $BuzzRoot rev-parse --short HEAD)"
Write-Host "Remote:          $(git -C $BuzzRoot remote get-url origin)"
Write-Host "See third_party/buzz/BONY.md and .gitmodules"
