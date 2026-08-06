# Resolve Desktop human owner pubkey for buzz-acp observer streaming.
# Without owner + BUZZ_ACP_RELAY_OBSERVER, buzz-acp can run a full model turn
# but Desktop never sees agent text (only process logs).
function Set-RoomAgentOwner {
  param(
    [string]$BonyRoot = (Get-BonyRoot),
    [string]$DefaultOwner = "f00af46ffd12fd562ee9117e7c663503f7afeaa8bbe1222e358001af77c6f0c2"
  )
  if (-not [string]::IsNullOrEmpty($env:BUZZ_ACP_AGENT_OWNER)) {
    Write-Host "  owner: env ($($env:BUZZ_ACP_AGENT_OWNER.Substring(0,[Math]::Min(12,$env:BUZZ_ACP_AGENT_OWNER.Length)))…)"
  } else {
    $ownerFile = Join-Path $BonyRoot ".local-runtime\room-owner.pubkey"
    $owner = $null
    if (Test-Path $ownerFile) {
      $owner = (Get-Content $ownerFile -Raw -ErrorAction SilentlyContinue).Trim()
    }
    if (-not $owner) { $owner = $DefaultOwner }
    if ($owner -and $owner.Length -ge 64) {
      $env:BUZZ_ACP_AGENT_OWNER = $owner
      if (-not (Test-Path $ownerFile)) {
        New-Item -ItemType Directory -Force -Path (Split-Path $ownerFile) | Out-Null
        [System.IO.File]::WriteAllText($ownerFile, $owner)
      }
      Write-Host "  owner: $($owner.Substring(0,12))… (UI stream target)"
    } else {
      Write-Warning "  owner: MISSING — agent will think but Desktop will not show replies"
    }
  }
  # Default-off in buzz-acp; without this, turns complete with no channel output.
  if ([string]::IsNullOrEmpty($env:BUZZ_ACP_RELAY_OBSERVER)) {
    $env:BUZZ_ACP_RELAY_OBSERVER = "true"
  }
  Write-Host "  relay_observer: $($env:BUZZ_ACP_RELAY_OBSERVER)"
}
