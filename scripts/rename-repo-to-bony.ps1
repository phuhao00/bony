# Already applied: phuhao00/bony-build -> phuhao00/bony
# Re-point local origin if you clone with an old URL.
param(
  [string]$Owner = "phuhao00",
  [string]$New = "bony"
)
$ErrorActionPreference = "Stop"
$newFull = "$Owner/$New"
Write-Host "Pointing origin at https://github.com/$newFull ..."
if (Test-Path .git) {
  git remote set-url origin "https://github.com/$newFull.git"
  git remote set-url --push origin "git@github.com:$newFull.git" 2>$null
  git remote -v
}
Write-Host "OK: https://github.com/$newFull"
