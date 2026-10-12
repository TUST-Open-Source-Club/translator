# Installer for the translate Windows release zip: installs translate.exe into
# %LOCALAPPDATA%\Programs\qwen-translate (added to the user PATH) and the bundled
# model (if any) into %LOCALAPPDATA%\qwen-translate.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\install.ps1
$ErrorActionPreference = 'Stop'

# This script lives in <package>\scripts\
$Root = Split-Path -Parent $PSScriptRoot
$BinDir = Join-Path $env:LOCALAPPDATA 'Programs\qwen-translate'
$DataDir = Join-Path $env:LOCALAPPDATA 'qwen-translate'
New-Item -ItemType Directory -Force -Path $BinDir, $DataDir | Out-Null

Copy-Item -Force (Join-Path $Root 'translate.exe') $BinDir
Write-Host "Installed: $BinDir\translate.exe"

# The -with-model packages carry both the model and the mmproj projector
$models = @(Get-ChildItem -Path (Join-Path $Root 'model') -Filter *.gguf -ErrorAction SilentlyContinue)
if ($models.Count -gt 0) {
  foreach ($m in $models) {
    Copy-Item -Force $m.FullName $DataDir
    Write-Host "Installed model: $DataDir\$($m.Name)"
  }
} else {
  Write-Host 'This package has no bundled model; it will be auto-downloaded on first run (via ModelScope).'
}

# Add the install dir to the user PATH. Read the raw registry value so entries like
# %USERPROFILE%\... stay unexpanded.
$envKey = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
$userPath = [string]$envKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
if (($userPath -split ';') -notcontains $BinDir) {
  $newPath = if ($userPath) { $userPath.TrimEnd(';') + ';' + $BinDir } else { $BinDir }
  $envKey.SetValue('Path', $newPath, [Microsoft.Win32.RegistryValueKind]::ExpandString)
  # Setting any user variable broadcasts WM_SETTINGCHANGE, so new terminals see the new PATH
  [Environment]::SetEnvironmentVariable('QWEN_TRANSLATE_REFRESH', '1', 'User')
  [Environment]::SetEnvironmentVariable('QWEN_TRANSLATE_REFRESH', $null, 'User')
  Write-Host "Added $BinDir to the user PATH (open a new terminal to use 'translate')"
}
$envKey.Close()

Write-Host "Browser extension: run $Root\scripts\install-native-host.ps1, then load $Root\firefox (Firefox) or $Root\chrome (Chrome/Edge); see README"
