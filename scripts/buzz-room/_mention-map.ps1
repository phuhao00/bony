# Build BUZZ_ACP_MENTION_MAP from scripts/buzz-room/keys/*.json so auto-posted
# text like "@ZeroClaw …" gets real Nostr p-tags (subscribe=mentions needs them).
function Set-RoomAgentMentionMap {
  param([string]$KeysDir = (Join-Path $PSScriptRoot "keys"))
  if (-not (Test-Path $KeysDir)) { return }
  $parts = @()
  $aliases = @{
    openmontage = @('OpenMontage','OpenMontage Agent','openmontage','Open Montage')
    docsmith    = @('DocSmith','Doc Smith','docsmith','文档','文档助手')
    unity       = @('Unity','Unity Agent','unity')
    zeroclaw    = @('ZeroClaw','zeroclaw','Zero Claw')
    grok        = @('Grok','grok')
  }
  Get-ChildItem -Path $KeysDir -Filter '*.json' -ErrorAction SilentlyContinue | ForEach-Object {
    try {
      $k = Get-Content $_.FullName -Raw | ConvertFrom-Json
      $pk = $null
      if ($k.public_key_hex) { $pk = [string]$k.public_key_hex }
      elseif ($k.pubkey) { $pk = [string]$k.pubkey }
      elseif ($k.public_key) { $pk = [string]$k.public_key }
      if (-not $pk -or $pk.Length -ne 64) { return }
      $stem = $_.BaseName.ToLowerInvariant()
      $names = @($stem)
      if ($aliases.ContainsKey($stem)) { $names += $aliases[$stem] }
      foreach ($n in ($names | Select-Object -Unique)) {
        $parts += ("{0}:{1}" -f $n, $pk.ToLowerInvariant())
      }
    } catch {}
  }
  if ($parts.Count -gt 0) {
    $env:BUZZ_ACP_MENTION_MAP = ($parts -join ',')
    Write-Host "  mention_map entries: $($parts.Count)"
  }
}
