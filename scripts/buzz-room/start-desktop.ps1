# Launch Buzz Desktop from bony-build only (third_party/buzz).
#
# Why it used to feel "always compiling":
#  - full sidecar cargo every start
#  - desktop used a *second* target/ directory → recompiled all path deps
#  - tauri dev always invokes cargo even when nothing changed
#
# Now:
#  - shared CARGO_TARGET_DIR with Buzz workspace
#  - sidecars built only when missing (-ForceSidecars to rebuild)
#  - -Fast: run prebuilt desktop binary + vite (seconds if already built)
#  - first full compile: scripts/buzz-room/build-desktop.ps1 (once)
param(
  [string]$BuzzRoot = "",
  [string]$RelayUrl = "ws://localhost:3000",
  [int]$VitePort = 1420,
  [switch]$Standalone,
  [switch]$ForceSidecars,
  [switch]$Fast,          # prefer prebuilt exe + vite only
  [switch]$TauriDev       # force classic `tauri dev` (slower but hot-reload Rust)
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
. (Join-Path $PSScriptRoot "_desktop-build.ps1")

if (-not $BuzzRoot) { $BuzzRoot = Get-BuzzRoot }
$BonyRoot = Get-BonyRoot
$Desktop = Join-Path $BuzzRoot "desktop"
$SrcTauri = Join-Path $Desktop "src-tauri"
$BinDir = Join-Path $SrcTauri "binaries"
$RuntimeDir = Join-Path $BonyRoot ".local-runtime"
New-Item -ItemType Directory -Force -Path $RuntimeDir, $BinDir | Out-Null

if (-not (Test-Path (Join-Path $Desktop "package.json"))) {
  throw "Buzz desktop missing at $Desktop"
}

Remove-Item Env:RUSTUP_TOOLCHAIN -ErrorAction SilentlyContinue
Set-FastDesktopCargoEnv -BuzzRoot $BuzzRoot
Enable-DesktopBuildEnv

if (-not $Standalone) {
  Write-Host "==> Relay health"
  $ok = $false
  try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:3000/health" -UseBasicParsing -TimeoutSec 3
    if ($r.StatusCode -eq 200) { $ok = $true }
  } catch {}
  if (-not $ok) {
    throw "Relay not up. Run: powershell -File scripts/buzz-room/start-room-stack.ps1 -SkipBuild -SkipGrok"
  }
  Write-Host "    OK"
}

$target = Get-HostTarget
Write-Host "==> Sidecars"
Ensure-Sidecars -BuzzRoot $BuzzRoot -BinDir $BinDir -Target $target -Force:$ForceSidecars

Write-Host "==> Frontend deps"
Set-Location $Desktop
if (-not (Test-Path "node_modules")) {
  Write-Host "    pnpm install (first time only)..."
  pnpm install
  if ($LASTEXITCODE -ne 0) { throw "pnpm install failed" }
} else {
  Write-Host "    node_modules OK"
}

$env:BUZZ_RELAY_URL = $RelayUrl
# Always pair HTTP base with WS for local-only stacks (avoids leftover wss://…buzz.xyz).
if ($RelayUrl -match '^ws://(localhost|127\.0\.0\.1)') {
  $httpHost = $RelayUrl -replace '^ws://', 'http://'
  $env:BUZZ_RELAY_HTTP = $httpHost
} elseif (-not $env:BUZZ_RELAY_HTTP) {
  $env:BUZZ_RELAY_HTTP = ($RelayUrl -replace '^wss://', 'https://' -replace '^ws://', 'http://')
}
if ($RelayUrl -match 'communities\.buzz\.xyz|blox\.sqprod|buzz\.xyz') {
  throw "Refusing production relay URL: $RelayUrl — use ws://localhost:3000 for bony local stack"
}
$env:BUZZ_VITE_PORT = "$VitePort"
$env:VITE_PORT = "$VitePort"
$env:BUZZ_DEV_KEYRING_SERVICE = "buzz-desktop-dev.bony"
# Isolate local monorepo Desktop from any previously installed official Buzz profile
$env:BUZZ_DESKTOP_INSTANCE_ID = "bony-local"
Remove-Item Env:BUZZ_PRIVATE_KEY -ErrorAction SilentlyContinue

$configPath = Join-Path $RuntimeDir "tauri.dev.override.json"
$configJson = @"
{
  "build": {
    "devUrl": "http://localhost:$VitePort",
    "beforeDevCommand": "pnpm exec vite --port $VitePort --strictPort"
  },
  "identifier": "xyz.block.buzz.app.dev",
  "productName": "Buzz Dev"
}
"@
[System.IO.File]::WriteAllText($configPath, $configJson)

$exe = Get-DesktopExe -BuzzRoot $BuzzRoot
$wantFast = $Fast -or (-not $TauriDev -and $exe)

if ($wantFast -and $exe) {
  Write-Host "==> Fast path: vite + existing binary"
  Write-Host "    $exe"
  Write-Host "    (Rust hot-reload needs: -TauriDev ; full rebuild: build-desktop.ps1)"
  $viteLog = Join-Path $RuntimeDir "vite.log"
  $vite = Start-Process -FilePath "pnpm.cmd" `
    -ArgumentList @("exec", "vite", "--port", "$VitePort", "--strictPort") `
    -WorkingDirectory $Desktop -WindowStyle Hidden -PassThru `
    -RedirectStandardOutput $viteLog -RedirectStandardError (Join-Path $RuntimeDir "vite.err")
  $vite.Id | Set-Content (Join-Path $RuntimeDir "vite.pid")

  Write-Host "    waiting for http://localhost:$VitePort ..."
  $ready = $false
  for ($i = 0; $i -lt 60; $i++) {
    try {
      $null = Invoke-WebRequest -Uri "http://localhost:$VitePort" -UseBasicParsing -TimeoutSec 1
      $ready = $true
      break
    } catch { Start-Sleep -Milliseconds 500 }
  }
  if (-not $ready) {
    Stop-Process -Id $vite.Id -Force -ErrorAction SilentlyContinue
    throw "Vite failed to start. See $viteLog / vite.err"
  }

  $env:TAURI_DEV = "1"
  $env:BUZZ_RELAY_URL = $RelayUrl
  if ($env:BUZZ_RELAY_HTTP) { } else { $env:BUZZ_RELAY_HTTP = "http://localhost:3000" }
  Write-Host "    local-only relay: $($env:BUZZ_RELAY_URL)  http: $($env:BUZZ_RELAY_HTTP)"
  Write-Host "    launching desktop UI (cwd=exe dir for shared DLLs)..."
  # Non-blocking GUI so this script can return while app stays open
  $exeDir = Split-Path $exe -Parent
  $app = Start-Process -FilePath $exe -WorkingDirectory $exeDir -PassThru
  $app.Id | Set-Content (Join-Path $RuntimeDir "desktop.pid")
  Write-Host "    desktop pid=$($app.Id)"
  Write-Host "    vite pid=$($vite.Id) (left running; stop via stop or kill)"
  exit 0
}

if (-not $exe) {
  Write-Host ""
  Write-Host "No buzz-desktop.exe under $($env:CARGO_TARGET_DIR)\debug\"
  Write-Host "Run ONCE: powershell -File scripts/buzz-room/build-desktop.ps1"
  if (-not $TauriDev) {
    throw "Desktop binary missing. Refusing tauri dev fallback (build-gate). Use build-desktop.ps1 first, or pass -TauriDev explicitly."
  }
  Write-Host "    -TauriDev requested: allowing tauri dev (will compile)"
}

Write-Host "==> tauri dev (cargo may run)"
Write-Host "    shared target: $($env:CARGO_TARGET_DIR)"
Set-Location $Desktop
$log = Join-Path $RuntimeDir "desktop.log"
pnpm exec tauri dev --config $configPath 2>&1 | Tee-Object -FilePath $log
