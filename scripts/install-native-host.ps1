# Register the native messaging host (Windows) for Firefox and the Chromium family
# (Chrome, Edge, Chromium, Brave). Per-user (HKCU): no administrator rights needed.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\install-native-host.ps1 [-Binary <translate.exe>] [-Uninstall]
param(
  [string]$Binary,
  [switch]$Uninstall
)
$ErrorActionPreference = 'Stop'

$HostName = 'qwen_translate'
$FirefoxExtId = 'qwen-translate@local'
# Fixed by the "key" in chrome\manifest.json
$ChromeExtId = 'bfkmmaobjaldoelkdphajjagomjehbpc'

$DataDir = Join-Path $env:LOCALAPPDATA 'qwen-translate'
$FirefoxManifest = Join-Path $DataDir 'native-host-firefox.json'
$ChromeManifest = Join-Path $DataDir 'native-host-chrome.json'
$Registrations = @(
  @{ Key = 'Mozilla'; Manifest = $FirefoxManifest },
  @{ Key = 'Google\Chrome'; Manifest = $ChromeManifest },
  @{ Key = 'Microsoft\Edge'; Manifest = $ChromeManifest },
  @{ Key = 'Chromium'; Manifest = $ChromeManifest },
  @{ Key = 'BraveSoftware\Brave-Browser'; Manifest = $ChromeManifest }
)

if ($Uninstall) {
  foreach ($r in $Registrations) {
    $key = "HKCU:\Software\$($r.Key)\NativeMessagingHosts\$HostName"
    if (Test-Path $key) {
      Remove-Item -Path $key -Recurse -Force
      Write-Host "Removed: $key"
    }
  }
  Remove-Item -Path $FirefoxManifest, $ChromeManifest -Force -ErrorAction SilentlyContinue
  exit 0
}

$Root = Split-Path -Parent $PSScriptRoot
if (-not $Binary) {
  $onPath = Get-Command translate.exe -ErrorAction SilentlyContinue
  $candidates = @(
    (Join-Path $Root 'target\release\translate.exe'),
    $(if ($onPath) { $onPath.Source }),
    (Join-Path $env:LOCALAPPDATA 'Programs\qwen-translate\translate.exe'),
    (Join-Path $Root 'translate.exe')
  )
  $Binary = $candidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
  if (-not $Binary) {
    throw 'translate.exe not found; run cargo build --release first or pass -Binary <path>'
  }
}
$Binary = (Resolve-Path $Binary).Path

New-Item -ItemType Directory -Force -Path $DataDir | Out-Null

function Write-HostManifest([string]$Path, [string]$AllowKey, [string]$AllowValue) {
  # The browser starts translate.exe directly (manifests cannot pass arguments);
  # it recognises the browser's arguments and switches to native messaging mode
  $manifest = [ordered]@{
    name        = $HostName
    description = 'Local translation native messaging host (Qwen)'
    path        = $Binary
    type        = 'stdio'
  }
  $manifest[$AllowKey] = @($AllowValue)
  # UTF-8 without BOM: browsers reject a manifest starting with a BOM
  $json = ConvertTo-Json -InputObject $manifest -Depth 3
  [IO.File]::WriteAllText($Path, $json, (New-Object Text.UTF8Encoding $false))
}

Write-HostManifest $FirefoxManifest 'allowed_extensions' $FirefoxExtId
Write-HostManifest $ChromeManifest 'allowed_origins' "chrome-extension://$ChromeExtId/"

foreach ($r in $Registrations) {
  $key = "HKCU:\Software\$($r.Key)\NativeMessagingHosts\$HostName"
  New-Item -Path $key -Force -Value $r.Manifest | Out-Null
  Write-Host "Registered: $key -> $($r.Manifest)"
}

Write-Host "Native host: $Binary"
Write-Host 'Next:'
Write-Host "  Firefox: about:debugging#/runtime/this-firefox -> Load Temporary Add-on -> $Root\firefox\manifest.json"
Write-Host "  Chrome/Edge: chrome://extensions (edge://extensions) -> Developer mode -> Load unpacked -> $Root\chrome"
