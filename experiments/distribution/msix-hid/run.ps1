# Packages the HID probe as a full-trust MSIX, installs it on this machine
# and runs it with and without package identity.
#
#   pwsh experiments/distribution/msix-hid/run.ps1 -Work <dir> -Results <dir>
#
# Meant for a disposable Windows runner (distribute.yml, job
# experiment-msix-hid, manual only). It creates a throwaway self-signed
# code signing certificate, trusts it in LocalMachine\TrustedPeople (needs
# an elevated shell), installs the package for the current user, and
# removes the package and the certificate at the end. No Store, no
# Partner Center, no DeckPilot identity.
param(
  [Parameter(Mandatory = $true)] [string] $Work,
  [Parameter(Mandatory = $true)] [string] $Results
)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$root = (Resolve-Path (Join-Path $here '../../..')).Path
New-Item -ItemType Directory -Force -Path $Work, $Results | Out-Null

function Find-Kit([string] $tool) {
  $found = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64' -Filter $tool -Recurse -ErrorAction SilentlyContinue |
    Sort-Object FullName -Descending | Select-Object -First 1
  if (-not $found) { throw "$tool not found in the Windows SDK" }
  return $found.FullName
}

cargo build --release --locked --manifest-path (Join-Path $here 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
$exe = Join-Path $here 'target\release\msix-hid.exe'
if ($env:CARGO_TARGET_DIR) { $exe = Join-Path $env:CARGO_TARGET_DIR 'release\msix-hid.exe' }

# Control: the same executable, unpackaged.
& $exe unpackaged (Join-Path $Results 'unpackaged.txt') | Out-Null

# The package layout: the executable, the manifest and the three logos
# the manifest names (DeckPilot's icons, only as placeholders).
$layout = Join-Path $Work 'layout'
New-Item -ItemType Directory -Force -Path (Join-Path $layout 'Assets') | Out-Null
Copy-Item $exe $layout
Copy-Item (Join-Path $here 'AppxManifest.xml') $layout
$icons = Join-Path $root 'apps\desktop\src-tauri\icons'
Copy-Item (Join-Path $icons '128x128.png') (Join-Path $layout 'Assets\Square150x150Logo.png')
Copy-Item (Join-Path $icons '32x32.png') (Join-Path $layout 'Assets\Square44x44Logo.png')
Copy-Item (Join-Path $icons '32x32.png') (Join-Path $layout 'Assets\StoreLogo.png')

$msix = Join-Path $Work 'msix-hid.msix'
& (Find-Kit 'makeappx.exe') pack /o /d $layout /p $msix | Tee-Object -FilePath (Join-Path $Results 'makeappx.txt')
if ($LASTEXITCODE -ne 0) { throw 'makeappx failed' }

# A throwaway certificate whose subject is the manifest's Publisher.
$password = [guid]::NewGuid().ToString()
$secure = ConvertTo-SecureString $password -AsPlainText -Force
$cert = New-SelfSignedCertificate -Type Custom -Subject 'CN=DeckPilot MSIX experiment' -KeyUsage DigitalSignature `
  -FriendlyName 'DeckPilot MSIX experiment (throwaway)' -CertStoreLocation 'Cert:\CurrentUser\My' `
  -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
$pfx = Join-Path $Work 'throwaway.pfx'
Export-PfxCertificate -Cert $cert -FilePath $pfx -Password $secure | Out-Null
$cer = Join-Path $Work 'throwaway.cer'
Export-Certificate -Cert $cert -FilePath $cer | Out-Null
Import-Certificate -FilePath $cer -CertStoreLocation 'Cert:\LocalMachine\TrustedPeople' | Out-Null

$installed = $false
try {
  & (Find-Kit 'signtool.exe') sign /fd SHA256 /f $pfx /p $password $msix | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'signtool failed' }
  Add-AppxPackage -Path $msix
  $installed = $true
  $package = Get-AppxPackage -Name 'DeckPilotExperiment.MsixHid'
  "package = $($package.PackageFullName)" | Out-File (Join-Path $Results 'package.txt') -Encoding utf8
  "install_location = $($package.InstallLocation)" | Out-File (Join-Path $Results 'package.txt') -Append -Encoding utf8

  # With package identity: through the execution alias.
  $alias = Join-Path $env:LOCALAPPDATA 'Microsoft\WindowsApps\deckpilot-msix-hid.exe'
  $report = Join-Path $Results 'packaged.txt'
  $process = Start-Process -FilePath $alias -ArgumentList @('packaged', $report) -Wait -PassThru -NoNewWindow
  "alias_exit_code = $($process.ExitCode)" | Out-File (Join-Path $Results 'package.txt') -Append -Encoding utf8
} finally {
  if ($installed) { Remove-AppxPackage -Package (Get-AppxPackage -Name 'DeckPilotExperiment.MsixHid').PackageFullName }
  Get-ChildItem 'Cert:\LocalMachine\TrustedPeople', 'Cert:\CurrentUser\My' |
    Where-Object { $_.Subject -eq 'CN=DeckPilot MSIX experiment' } | Remove-Item
  Remove-Item $pfx, $cer -ErrorAction SilentlyContinue
}

foreach ($name in 'unpackaged', 'packaged') {
  $file = Join-Path $Results "$name.txt"
  if (-not (Test-Path $file)) { Write-Host "::warning::no $name report"; continue }
  Write-Host "== $name"
  Get-Content $file | Select-String -Pattern '^(label|package_full_name|hid_init|hid_devices|elgato_devices) ' | ForEach-Object { $_.Line }
  $opened = (Select-String -Path $file -Pattern '^device\[\d+\] open = opened').Count
  $failed = (Select-String -Path $file -Pattern '^device\[\d+\] open = failed').Count
  Write-Host "opened = $opened, failed = $failed"
}
