# Build Buzz Desktop from bony-build monorepo root (single Cargo workspace).
#
# After first success:
#   powershell -File scripts/buzz-room/start-desktop.ps1
# launches the prebuilt binary without recompiling.
param(
  [string]$BuzzRoot = "",
  [switch]$Release,
  [switch]$ForceSidecars,
  [switch]$NoLocalStt,    # omit sherpa-onnx STT (default: STT ON via shared DLLs)
  [switch]$KeepDupTarget  # keep third_party/buzz/target (default: delete legacy)
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
. (Join-Path $PSScriptRoot "_desktop-build.ps1")

$RepoRoot = Get-BonyRoot
if (-not $BuzzRoot) { $BuzzRoot = Get-BuzzRoot }
$Desktop = Join-Path $BuzzRoot "desktop"
$SrcTauri = Join-Path $Desktop "src-tauri"
$BinDir = Join-Path $SrcTauri "binaries"
$RuntimeDir = Join-Path $RepoRoot ".local-runtime"
New-Item -ItemType Directory -Force -Path $RuntimeDir | Out-Null

Write-Host "==> Desktop build env (monorepo root)"
Enable-DesktopBuildEnv
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
  throw "cmake required. winget install Kitware.CMake"
}

Set-FastDesktopCargoEnv -BuzzRoot $BuzzRoot
Write-Host "    CARGO_TARGET_DIR=$($env:CARGO_TARGET_DIR)  jobs=$($env:CARGO_BUILD_JOBS)"
Write-Host "    cargo -p from $RepoRoot"

if (-not $KeepDupTarget) {
  Write-Host "==> Remove legacy buzz/desktop targets (use root target only)"
  Remove-LegacyDesktopTarget -BuzzRoot $BuzzRoot -Force
} else {
  Remove-LegacyDesktopTarget -BuzzRoot $BuzzRoot
}

$target = Get-HostTarget
Write-Host "==> Sidecars"
Ensure-Sidecars -BuzzRoot $BuzzRoot -BinDir $BinDir -Target $target -Force:$ForceSidecars

Write-Host "==> Frontend deps"
Set-Location $Desktop
if (-not (Test-Path "node_modules")) {
  pnpm install
  if ($LASTEXITCODE -ne 0) { throw "pnpm install failed" }
} else {
  Write-Host "    node_modules OK"
}

# Default features: system-keyring + local-stt (shared sherpa DLLs)
$cargoArgs = @("build", "-p", "buzz-desktop")
if ($NoLocalStt) {
  Write-Host "==> local-stt OFF (--no-default-features + system-keyring only)"
  $cargoArgs += @("--no-default-features", "--features", "system-keyring")
} else {
  Write-Host "==> default features (system-keyring + local-stt via shared sherpa DLLs)"
}
if ($Release) { $cargoArgs += "--release" }

$profile = if ($Release) { "release" } else { "debug" }
Write-Host "==> cargo $($cargoArgs -join ' ')  ($profile) -> $($env:CARGO_TARGET_DIR)"
Set-Location $RepoRoot
$sw = [Diagnostics.Stopwatch]::StartNew()
& cargo @cargoArgs
$code = $LASTEXITCODE
$sw.Stop()
Write-Host "    cargo finished in $([int]$sw.Elapsed.TotalSeconds)s (exit $code)"
if ($code -ne 0) {
  throw "Desktop cargo build failed. See messages above."
}

$exe = Get-DesktopExe -BuzzRoot $BuzzRoot
if ($exe) {
  Write-Host "OK: $exe"
  Write-Host "Daily start (no compile): powershell -File scripts/buzz-room/start-desktop.ps1"
} else {
  Write-Host "Build finished; expected exe under $($env:CARGO_TARGET_DIR)\$profile\"
}
