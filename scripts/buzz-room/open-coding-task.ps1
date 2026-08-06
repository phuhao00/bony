# Detached launch of a Bony Build desktop window pre-seeded with a coding task.
param(
  [Parameter(Mandatory = $true)]
  [string]$Prompt,
  [string]$RepoPath = "",
  [string]$Title = "",
  [string]$BonyRoot = "",
  [string]$PromptFile = "",
  [string]$ExePath = ""
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
if (-not $BonyRoot) { $BonyRoot = Get-BonyRoot }
if (-not $RepoPath) { $RepoPath = $BonyRoot }

if ($PromptFile -and (Test-Path $PromptFile)) {
  $Prompt = Get-Content -Raw -Encoding UTF8 $PromptFile
}
if ([string]::IsNullOrWhiteSpace($Prompt)) {
  throw "Prompt is empty"
}

function Find-BonyBuild {
  param([string]$Root, [string]$Explicit)
  if ($Explicit -and (Test-Path $Explicit)) { return (Resolve-Path $Explicit).Path }
  foreach ($prof in @("release", "debug")) {
    $p = Join-Path $Root "target\$prof\bony-build.exe"
    if (Test-Path $p) { return (Resolve-Path $p).Path }
  }
  $cmd = Get-Command bony-build.exe -ErrorAction SilentlyContinue
  if ($cmd) { return $cmd.Source }
  return $null
}

$exe = Find-BonyBuild -Root $BonyRoot -Explicit $ExePath
if (-not $exe) {
  throw "bony-build.exe not found under $BonyRoot\target\{release,debug}. Run: cargo build -p bony-build --release"
}

$tmpDir = Join-Path $env:TEMP "bony-coding-tasks"
New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null
$seed = Join-Path $tmpDir ("seed-{0}.txt" -f [guid]::NewGuid().ToString("N"))
# UTF-8 without BOM for Rust fs::read_to_string friendliness
$utf8 = New-Object System.Text.UTF8Encoding $false
[System.IO.File]::WriteAllText($seed, $Prompt.Trim(), $utf8)

$argList = @(
  "--cwd", $RepoPath,
  "--seed-prompt-file", $seed
)
if (-not [string]::IsNullOrWhiteSpace($Title)) {
  $argList += @("--task-title", $Title)
}

$proc = Start-Process -FilePath $exe -ArgumentList $argList -WorkingDirectory $RepoPath -PassThru -WindowStyle Normal
Write-Host ("started bony-build pid={0} cwd={1} seed={2}" -f $proc.Id, $RepoPath, $seed)
Write-Output ("pid={0}`ncwd={1}`nseed={2}`nexe={3}" -f $proc.Id, $RepoPath, $seed, $exe)
