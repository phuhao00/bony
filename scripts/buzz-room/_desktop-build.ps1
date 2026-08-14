# Shared Desktop build helpers (dot-source after _paths.ps1).
# Monorepo: one Cargo workspace at bony-build root. Target dir is repo-root/target.

function Enable-DesktopBuildEnv {
  foreach ($cmakeBin in @(
      "C:\Program Files\CMake\bin",
      "C:\Program Files (x86)\CMake\bin"
    )) {
    if (Test-Path (Join-Path $cmakeBin "cmake.exe")) {
      $env:Path = "$cmakeBin;$env:Path"
      break
    }
  }

  $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
  if (-not (Test-Path $vswhere)) { return }
  $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
  if (-not $vs) { return }
  $vcvars = Join-Path $vs "VC\Auxiliary\Build\vcvars64.bat"
  if (-not (Test-Path $vcvars)) { return }

  $tmp = [IO.Path]::GetTempFileName() + ".bat"
  @"
@echo off
call "$vcvars" >nul 2>&1
set
"@ | Set-Content -Encoding ascii $tmp
  cmd /c "`"$tmp`"" | ForEach-Object {
    if ($_ -match '^([^=]+)=(.*)$') {
      $name = $Matches[1]
      $val = $Matches[2]
      if ($name -match '^[A-Za-z_][A-Za-z0-9_]*$') {
        [Environment]::SetEnvironmentVariable($name, $val, 'Process')
      }
    }
  }
  Remove-Item $tmp -Force -ErrorAction SilentlyContinue
}

function Get-HostTarget {
  try {
    $hostLine = & rustc -vV 2>&1 | Select-String "host:"
    if ($hostLine -match "host:\s*(\S+)") { return $Matches[1] }
  } catch {}
  return "x86_64-pc-windows-msvc"
}

function Get-SharedTargetDir {
  # Always monorepo-root/target (single Cargo workspace).
  return (Join-Path (Get-BonyRoot) "target")
}

function Set-FastDesktopCargoEnv {
  param([string]$BuzzRoot)
  Remove-Item Env:RUSTUP_TOOLCHAIN -ErrorAction SilentlyContinue
  Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
  $env:CARGO_TARGET_DIR = Get-SharedTargetDir
  $env:CARGO_INCREMENTAL = "1"
  $jobs = [Environment]::ProcessorCount
  if ($jobs -lt 1) { $jobs = 8 }
  $env:CARGO_BUILD_JOBS = "$jobs"
  $env:CMAKE_POLICY_VERSION_MINIMUM = "3.5"
}

function Get-DesktopExe {
  param([string]$BuzzRoot)
  $shared = Get-SharedTargetDir
  foreach ($prof in @("debug", "release")) {
    $p = Join-Path $shared "$prof\buzz-desktop.exe"
    if (Test-Path $p) { return $p }
  }
  return $null
}

function Ensure-Sidecars {
  param(
    [string]$BuzzRoot,
    [string]$BinDir,
    [string]$Target,
    [switch]$Force
  )
  $need = @(
    "buzz-acp",
    "buzz-agent",
    "buzz-dev-mcp",
    "git-credential-nostr",
    "buzz"
  )
  $pkgs = @{
    "buzz-acp" = "buzz-acp"
    "buzz-agent" = "buzz-agent"
    "buzz-dev-mcp" = "buzz-dev-mcp"
    "git-credential-nostr" = "git-credential-nostr"
    "buzz" = "buzz-cli"
  }

  New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
  $shared = Get-SharedTargetDir
  $missing = @()
  foreach ($n in $need) {
    $src = Join-Path $shared "debug\$n.exe"
    $dst = Join-Path $BinDir "$n-$Target.exe"
    if ($Force -or -not (Test-Path $src) -or -not (Test-Path $dst)) {
      $missing += $n
    }
  }

  if ($missing.Count -eq 0) {
    Write-Host "    sidecars OK (skip cargo)"
    foreach ($n in $need) {
      $src = Join-Path $shared "debug\$n.exe"
      $dst = Join-Path $BinDir "$n-$Target.exe"
      if (-not (Test-Path $dst)) {
        Copy-Item $src $dst -Force
      } elseif ((Get-Item $src).LastWriteTime -gt (Get-Item $dst).LastWriteTime) {
        Copy-Item $src $dst -Force
      }
    }
    return
  }

  Write-Host "    cargo build missing sidecars: $($missing -join ', ')"
  $pkgSet = New-Object System.Collections.Generic.HashSet[string]
  foreach ($n in $missing) { [void]$pkgSet.Add($pkgs[$n]) }
  $pkgArgs = @()
  foreach ($p in $pkgSet) { $pkgArgs += @("-p", $p) }

  $repoRoot = Get-BonyRoot
  Push-Location $repoRoot
  try {
    Set-FastDesktopCargoEnv -BuzzRoot $BuzzRoot
    & cargo build @pkgArgs
    if ($LASTEXITCODE -ne 0) { throw "sidecar cargo build failed" }
  } finally {
    Pop-Location
  }

  foreach ($n in $need) {
    $src = Join-Path $shared "debug\$n.exe"
    if (-not (Test-Path $src)) { throw "missing $src" }
    $dst = Join-Path $BinDir "$n-$Target.exe"
    Copy-Item $src $dst -Force
  }
}

function Remove-LegacyDesktopTarget {
  param(
    [string]$BuzzRoot,
    [switch]$Force
  )
  $legacy1 = Join-Path $BuzzRoot "desktop\src-tauri\target"
  $legacy2 = Join-Path $BuzzRoot "target"
  foreach ($legacy in @($legacy1, $legacy2)) {
    if (-not (Test-Path $legacy)) {
      Write-Host "    no legacy target at $legacy"
      continue
    }
    if (-not $Force) {
      Write-Host "    legacy target exists (use mono-repo root target instead): $legacy"
      continue
    }
    Write-Host "    removing legacy target $legacy ..."
    Remove-Item -LiteralPath $legacy -Recurse -Force -ErrorAction SilentlyContinue
    if (Test-Path $legacy) {
      Write-Warning "could not fully remove $legacy (file in use?)"
    } else {
      Write-Host "    legacy target removed"
    }
  }
}
