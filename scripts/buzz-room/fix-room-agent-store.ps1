# Restore lean managed-agents.json + re-inject seat nsecs so Desktop does not
# spawn DocSmith in setup-listener (empty key / keyring oversize → NotReady).
#
# Safe for local room stack. Does not print nsecs.
param(
  [switch]$AlsoDeleteWindowsCredentials
)
$ErrorActionPreference = "Stop"
. (Join-Path $PSScriptRoot "_paths.ps1")
$KeysDir = Join-Path $PSScriptRoot "keys"

$storePaths = @(
  (Join-Path $env:APPDATA "xyz.block.buzz.app\agents\managed-agents.json"),
  (Join-Path $env:APPDATA "xyz.block.buzz.app.dev\agents\managed-agents.json"),
  (Join-Path $env:APPDATA "xyz.block.buzz.app.bony-local\agents\managed-agents.json")
) | Select-Object -Unique

# Backup + rewrite lean seats only (via register when possible; fallback local).
Write-Host "==> Backup + lean rewrite of managed-agents stores"
$now = (Get-Date).ToUniversalTime().ToString("yyyyMMdd-HHmmss")
foreach ($p in $storePaths) {
  if (-not (Test-Path $p)) { continue }
  $bak = "$p.bak-$now"
  Copy-Item $p $bak -Force
  Write-Host "  backed up $p -> $bak ($((Get-Item $p).Length) bytes)"
}

# Prefer full register (directory + channel + store) when relay is up.
$register = Join-Path $PSScriptRoot "register-room-agents.ps1"
try {
  & $register -ReplaceStore
} catch {
  Write-Warning "register-room-agents failed ($_) — writing lean store from keys only"
  $roster = @()
  foreach ($id in @("grok","zeroclaw","unity","openmontage","docsmith")) {
    $kf = Join-Path $KeysDir "$id.json"
    if (-not (Test-Path $kf)) { continue }
    $k = Get-Content $kf -Raw | ConvertFrom-Json
    $sk = if ($k.nsec) { $k.nsec } elseif ($k.private_key) { $k.private_key } else { $null }
    $pk = [string]$k.public_key_hex
    if (-not $sk -or $pk.Length -ne 64) { continue }
    $name = switch ($id) {
      "grok" { "Grok" }
      "zeroclaw" { "ZeroClaw" }
      "unity" { "Unity" }
      "openmontage" { "OpenMontage" }
      "docsmith" { "DocSmith" }
    }
    $roster += [ordered]@{
      pubkey = $pk
      name = $name
      private_key_nsec = $sk
      relay_url = "ws://localhost:3000"
      acp_command = "buzz-acp"
      agent_command = "grok.cmd"
      agent_args = @("agent", "stdio")
      mcp_command = ""
      system_prompt = "Local room agent: $name"
      respond_to = "anyone"
      respond_to_allowlist = @()
      backend = @{ type = "local" }
      is_active = $true
      is_builtin = $false
      start_on_app_launch = $false
      auto_restart_on_config_change = $false
      parallelism = 1
      turn_timeout_seconds = 320
      created_at = (Get-Date).ToUniversalTime().ToString("o")
      updated_at = (Get-Date).ToUniversalTime().ToString("o")
      avatar_url = $null
      persona_id = $null
      runtime_pid = $null
    }
  }
  $json = $roster | ConvertTo-Json -Depth 8
  foreach ($p in $storePaths) {
    $dir = Split-Path $p -Parent
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    [System.IO.File]::WriteAllText($p, $json, [System.Text.UTF8Encoding]::new($false))
    Write-Host "  wrote lean $($roster.Count) seats -> $p"
  }
}

# Ensure every room seat still has inline nsec (Desktop may have stripped after
# a "successful" keyring write that later became unreadable / oversized).
Write-Host "==> Ensure inline nsecs for room seats"
foreach ($p in $storePaths) {
  if (-not (Test-Path $p)) { continue }
  $rawText = Get-Content $p -Raw -Encoding UTF8
  if ([string]::IsNullOrWhiteSpace($rawText)) {
    Write-Warning "  $p is empty — re-run register-room-agents.ps1"
    continue
  }
  $parsed = $rawText | ConvertFrom-Json
  $arr = @($parsed)
  if ($arr.Count -eq 0) {
    Write-Warning "  $p parsed empty — skip rewrite"
    continue
  }
  $changed = $false
  $newList = New-Object System.Collections.Generic.List[object]
  foreach ($a in $arr) {
    if ($null -eq $a) { continue }
    $pk = [string]$a.pubkey
    if ([string]::IsNullOrWhiteSpace($pk) -or $pk.Length -ne 64) { continue }
    $av = [string]$a.avatar_url
    if ($av.Length -gt 500) {
      $a | Add-Member -NotePropertyName avatar_url -NotePropertyValue $null -Force
      $changed = $true
    }
    $keyFile = $null
    foreach ($id in @("grok","zeroclaw","unity","openmontage","docsmith")) {
      $kf = Join-Path $KeysDir "$id.json"
      if (-not (Test-Path $kf)) { continue }
      $k = Get-Content $kf -Raw | ConvertFrom-Json
      if (([string]$k.public_key_hex).ToLowerInvariant() -eq $pk.ToLowerInvariant()) {
        $keyFile = $k
        break
      }
    }
    if ($keyFile) {
      $sk = if ($keyFile.nsec) { $keyFile.nsec } else { $keyFile.private_key }
      if ($sk -and [string]$a.private_key_nsec -ne [string]$sk) {
        $a | Add-Member -NotePropertyName private_key_nsec -NotePropertyValue $sk -Force
        $changed = $true
      }
      try { $a.start_on_app_launch = $false } catch {}
      try { $a.auto_restart_on_config_change = $false } catch {}
    }
    $newList.Add($a) | Out-Null
  }
  if ($newList.Count -eq 0) {
    Write-Warning "  $p: would empty the store — refuse to write"
    continue
  }
  if ($changed) {
    $json = $newList.ToArray() | ConvertTo-Json -Depth 8
    if ([string]::IsNullOrWhiteSpace($json) -or $json.Length -lt 100) {
      Write-Warning "  $p: ConvertTo-Json empty/short — refuse to write"
      continue
    }
    [System.IO.File]::WriteAllText($p, $json, [System.Text.UTF8Encoding]::new($false))
  }
  $size = (Get-Item $p).Length
  Write-Host ("  {0}  size={1}  agents={2}" -f $p, $size, $newList.Count)
}

if ($AlsoDeleteWindowsCredentials) {
  Write-Host "==> Delete bloated Windows keyring targets for buzz-desktop*"
  Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class BuzzCred {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  public struct CREDENTIAL {
    public int Flags;
    public int Type;
    public string TargetName;
    public string Comment;
    public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
    public int CredentialBlobSize;
    public IntPtr CredentialBlob;
    public int Persist;
    public int AttributeCount;
    public IntPtr Attributes;
    public string TargetAlias;
    public string UserName;
  }
  [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
  public static extern bool CredEnumerate(string Filter, int Flags, out int Count, out IntPtr Credentials);
  [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
  public static extern bool CredDelete(string Target, int Type, int Flags);
  [DllImport("advapi32.dll")]
  public static extern void CredFree(IntPtr Buffer);
  public static void DeleteBuzz() {
    IntPtr creds;
    int count;
    // Type 1 = CRED_TYPE_GENERIC. Filter may be null to enumerate all (needs flag 0x1 on some builds).
    if (!CredEnumerate(null, 0x1, out count, out creds) || count <= 0) {
      Console.WriteLine("  CredEnumerate failed or empty (err=" + Marshal.GetLastWin32Error() + ")");
      return;
    }
    int deleted = 0;
    for (int i = 0; i < count; i++) {
      IntPtr p = Marshal.ReadIntPtr(creds, i * IntPtr.Size);
      CREDENTIAL c = (CREDENTIAL)Marshal.PtrToStructure(p, typeof(CREDENTIAL));
      string t = c.TargetName ?? "";
      string u = c.UserName ?? "";
      if (t.IndexOf("buzz-desktop", StringComparison.OrdinalIgnoreCase) >= 0 ||
          u.IndexOf("buzz-desktop", StringComparison.OrdinalIgnoreCase) >= 0 ||
          t.IndexOf("keyring:Windows", StringComparison.OrdinalIgnoreCase) >= 0 && t.IndexOf("buzz", StringComparison.OrdinalIgnoreCase) >= 0) {
        if (CredDelete(t, c.Type, 0)) {
          Console.WriteLine("  deleted " + t + " type=" + c.Type + " blob=" + c.CredentialBlobSize);
          deleted++;
        } else {
          Console.WriteLine("  delete failed " + t + " err=" + Marshal.GetLastWin32Error());
        }
      }
    }
    CredFree(creds);
    Console.WriteLine("  deleted count=" + deleted);
  }
}
"@
  [BuzzCred]::DeleteBuzz()
}

Write-Host ""
Write-Host "Next:"
Write-Host "  1) Fully quit Buzz Desktop"
Write-Host "  2) powershell -File scripts/buzz-room/start-external-room-agents.ps1"
Write-Host "  3) Relaunch Desktop; do NOT Start room bot cards (external seats own them)"
Write-Host "  4) Bare human messages → Grok routes @DocSmith with p-tags; or use member picker"
