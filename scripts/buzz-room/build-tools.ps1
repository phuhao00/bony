# Build room-side binaries used by Buzz specialists.
param(
  [string]$BonyRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path,
  [string]$BuzzRoot = ""
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
$BonyRoot = Get-BonyRoot $BonyRoot
if (-not $BuzzRoot) {
  try { $BuzzRoot = Get-BuzzRoot } catch {
    Write-Host "Buzz missing — running setup-buzz.ps1 ..."
    & (Join-Path $PSScriptRoot "setup-buzz.ps1") -BonyRoot $BonyRoot
    $BuzzRoot = Get-BuzzRoot
  }
}

Write-Host "==> bony-room-tools-mcp"
Set-Location $BonyRoot
cargo build -p bony-room-tools-mcp --release
if ($LASTEXITCODE -ne 0) { throw "bony-room-tools-mcp build failed" }

Write-Host "==> bony-docs-tools-mcp"
cargo build -p bony-docs-tools-mcp --release
if ($LASTEXITCODE -ne 0) { throw "bony-docs-tools-mcp build failed" }

Write-Host "==> buzz-acp / buzz-cli / buzz-admin / buzz-dev-mcp / buzz-agent (Buzz tree)"
if (-not (Test-Path $BuzzRoot)) {
  throw "Buzz root missing ($BuzzRoot) — run scripts/buzz-room/setup-buzz.ps1"
}
# Buzz is a plain source tree, not its own Cargo workspace — it's a member of
# the single root workspace (see third_party/buzz/Cargo.toml). Build from the
# repo root so there's one workspace / one target/, per project rules.
Set-Location $BonyRoot
Remove-Item Env:RUSTUP_TOOLCHAIN -ErrorAction SilentlyContinue
Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
cargo build -p buzz-acp -p buzz-cli -p buzz-admin -p buzz-dev-mcp -p buzz-agent -p buzz-relay
if ($LASTEXITCODE -ne 0) { throw "buzz crate build failed" }

Write-Host "Done."
Write-Host "  room MCP: $BonyRoot\target\release\bony-room-tools-mcp.exe"
Write-Host "  docs MCP: $BonyRoot\target\release\bony-docs-tools-mcp.exe"
Write-Host "  buzz bins: $BonyRoot\target\debug\ (buzz-acp, buzz-agent, buzz-dev-mcp, buzz-admin, buzz, buzz-relay)"
Write-Host "Next: start-room-stack.ps1  (or start-infra + start-relay + start-external-room-agents)"
