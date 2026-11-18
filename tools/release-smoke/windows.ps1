# The published Windows installers on a GitHub-hosted Windows runner:
#
#   windows.ps1 install-msi   <label> <msi>        silent install (msiexec /qn, verbose log)
#   windows.ps1 uninstall-msi <label> <msi>
#   windows.ps1 install-nsis  <label> <setup.exe>  silent install (/S)
#   windows.ps1 uninstall-nsis <label>
#   windows.ps1 launch        <label>              first launch with an isolated profile
#   windows.ps1 gui           <label>              tauri-driver + msedgedriver, gui.mjs
#
# install-* write the installation directory to $env:SMOKE_WORK\<label>.dir
# and to GITHUB_ENV as SMOKE_INSTALL_DIR. Results go to
# $env:SMOKE_OUT\results.tsv (format: tools/release-smoke/common.sh).
#
# Isolation: USERPROFILE, HOME, APPDATA, LOCALAPPDATA, TEMP and
# DECKPILOT_DATA_DIR point into a new profile directory, and
# WEBVIEW2_USER_DATA_FOLDER keeps WebView2's profile there. Windows resolves
# the known folders (FOLDERID_RoamingAppData, ...) without those variables,
# so the real profile is watched and anything DeckPilot creates there is
# reported. The system key goes to Credential Manager, which cannot be
# redirected: the entries it adds are listed and deleted afterwards.
param(
  [Parameter(Mandatory = $true)][string]$Command,
  [Parameter(Mandatory = $true)][string]$Label,
  [string]$File
)
$ErrorActionPreference = 'Continue'
$out = $env:SMOKE_OUT
$work = $env:SMOKE_WORK
$platform = $env:SMOKE_PLATFORM
New-Item -ItemType Directory -Force -Path $out, $work | Out-Null

function Record([string]$Step, [string]$Result, [string]$Evidence) {
  $clean = ($Evidence -replace "[`t`r`n]+", ' ')
  if ($clean.Length -gt 900) { $clean = $clean.Substring(0, 900) }
  [IO.File]::AppendAllText((Join-Path $out 'results.tsv'), "$platform`t$Step`t$Result`t$clean`n")
  switch ($Result) {
    'pass' { Write-Host "PASS  ${Step}: $clean" }
    'fail' { Write-Host "::error title=${Step}::$clean" }
    default { Write-Host "::notice title=${Step}::not covered: $clean" }
  }
}

function Find-Install {
  $roots = @($env:ProgramFiles, ${env:ProgramFiles(x86)}, (Join-Path $env:LOCALAPPDATA 'Programs'), $env:LOCALAPPDATA)
  foreach ($root in $roots) {
    if (-not $root) { continue }
    foreach ($name in 'DeckPilot', 'deckpilot') {
      $candidate = Join-Path $root $name
      if (Test-Path (Join-Path $candidate 'deckpilot-desktop.exe')) { return (Resolve-Path $candidate).Path }
    }
  }
  $keys = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*'
  $entry = Get-ItemProperty $keys -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -like 'DeckPilot*' } | Select-Object -First 1
  if ($entry -and $entry.InstallLocation -and (Test-Path (Join-Path $entry.InstallLocation 'deckpilot-desktop.exe'))) { return $entry.InstallLocation.TrimEnd('\') }
  return $null
}

function Uninstall-Entry {
  $keys = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*'
  Get-ItemProperty $keys -ErrorAction SilentlyContinue | Where-Object { $_.DisplayName -like 'DeckPilot*' } |
    ForEach-Object { "$($_.DisplayName) $($_.DisplayVersion) [$($_.PSChildName)]" }
}

function Save-Dir([string]$Dir) {
  Set-Content -Path (Join-Path $work "$Label.dir") -Value $Dir -Encoding ascii
  if ($env:GITHUB_ENV) { "SMOKE_INSTALL_DIR=$Dir" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8 }
}

function Stop-DeckPilot {
  Get-Process deckpilot-desktop, deckpilot-agent, deckpilot-plugin-host -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  for ($i = 0; $i -lt 20; $i++) {
    if (-not (Get-Process deckpilot-desktop, deckpilot-agent -ErrorAction SilentlyContinue)) { return $true }
    Start-Sleep -Milliseconds 500
  }
  return -not (Get-Process deckpilot-desktop, deckpilot-agent -ErrorAction SilentlyContinue)
}

function DeckPilot-Credentials {
  (cmdkey /list) | Where-Object { $_ -match 'Target:.*DeckPilot' } | ForEach-Object { ($_ -replace '^\s*Target:\s*', '').Trim() }
}

function New-Profile([string]$Name) {
  $p = Join-Path $work "profile-$Name"
  if (Test-Path $p) { Remove-Item -Recurse -Force $p }
  foreach ($d in 'home', 'roaming', 'local', 'data', 'webview2', 'temp') { New-Item -ItemType Directory -Force -Path (Join-Path $p $d) | Out-Null }
  return $p
}

function Profile-Env([string]$P) {
  return @{
    USERPROFILE = (Join-Path $P 'home'); HOME = (Join-Path $P 'home')
    APPDATA = (Join-Path $P 'roaming'); LOCALAPPDATA = (Join-Path $P 'local')
    TEMP = (Join-Path $P 'temp'); TMP = (Join-Path $P 'temp')
    DECKPILOT_DATA_DIR = (Join-Path $P 'data')
    WEBVIEW2_USER_DATA_FOLDER = (Join-Path $P 'webview2')
  }
}

function Start-WithEnv([string]$Exe, $Env) {
  $psi = New-Object System.Diagnostics.ProcessStartInfo
  $psi.FileName = $Exe
  $psi.UseShellExecute = $false
  foreach ($k in $Env.Keys) { $psi.Environment[$k] = $Env[$k] }
  return [System.Diagnostics.Process]::Start($psi)
}

function Screenshot([string]$Name) {
  try {
    Add-Type -AssemblyName System.Windows.Forms, System.Drawing
    $bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
    $file = Join-Path $out "$Name.png"
    $bitmap.Save($file, [System.Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    return "$Name.png ($($bounds.Width)x$($bounds.Height))"
  } catch {
    return $null
  }
}

function Real-ProfileState {
  $paths = @(
    (Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'io.github.knittingcode.deckpilot'),
    (Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'io.github.knittingcode.deckpilot')
  )
  return $paths | Where-Object { Test-Path $_ }
}

function Real-ProfileLeaks([datetime]$Since) {
  $roots = @([Environment]::GetFolderPath('ApplicationData'), [Environment]::GetFolderPath('LocalApplicationData'), [Environment]::GetFolderPath('UserProfile'))
  $found = @()
  foreach ($root in $roots) {
    $found += Get-ChildItem -LiteralPath $root -Force -ErrorAction SilentlyContinue |
      Where-Object { $_.Name -like '*deckpilot*' -and $_.LastWriteTime -ge $Since } | ForEach-Object { $_.FullName }
  }
  return $found | Where-Object { $_ -and -not $_.StartsWith($work) } | Select-Object -Unique
}

$dirFile = Join-Path $work "$Label.dir"
switch ($Command) {
  'install-msi' {
    $log = Join-Path $out "$Label-msiexec.log"
    $p = Start-Process msiexec.exe -ArgumentList @('/i', "`"$File`"", '/qn', '/norestart', '/l*v', "`"$log`"") -Wait -PassThru
    $dir = Find-Install
    $success = (Select-String -Path $log -Pattern 'Installation success or error status: 0' -SimpleMatch -Quiet)
    if ($p.ExitCode -eq 0 -and $dir) {
      Record "${Label}: msiexec /i /qn" 'pass' "exit 0, installed to $dir; $(Uninstall-Entry); log $Label-msiexec.log$(if ($success) { ' (Installation success or error status: 0)' })"
      Save-Dir $dir
    } else {
      Record "${Label}: msiexec /i /qn" 'fail' "exit $($p.ExitCode), install dir '$dir'; $(Select-String -Path $log -Pattern 'error|failed' | Select-Object -Last 3)"
    }
  }
  'uninstall-msi' {
    $log = Join-Path $out "$Label-msiexec-uninstall.log"
    [void](Stop-DeckPilot)
    $p = Start-Process msiexec.exe -ArgumentList @('/x', "`"$File`"", '/qn', '/norestart', '/l*v', "`"$log`"") -Wait -PassThru
    $dir = Get-Content $dirFile -ErrorAction SilentlyContinue
    $left = if ($dir -and (Test-Path (Join-Path $dir 'deckpilot-desktop.exe'))) { 'deckpilot-desktop.exe still there' } else { '' }
    if ($p.ExitCode -eq 0 -and -not $left -and -not (Uninstall-Entry)) {
      Record "${Label}: msiexec /x /qn" 'pass' "exit 0; $dir removed; no uninstall entry left"
    } else {
      Record "${Label}: msiexec /x /qn" 'fail' "exit $($p.ExitCode); $left; entries: $(Uninstall-Entry)"
    }
  }
  'install-nsis' {
    $p = Start-Process -FilePath $File -ArgumentList '/S' -Wait -PassThru
    # The NSIS installer may return before its files are in place.
    $dir = $null
    for ($i = 0; $i -lt 60 -and -not $dir; $i++) { $dir = Find-Install; if (-not $dir) { Start-Sleep 1 } }
    if ($p.ExitCode -eq 0 -and $dir) {
      Record "${Label}: setup.exe /S" 'pass' "exit 0, installed to $dir; $(Uninstall-Entry)"
      Save-Dir $dir
    } else {
      Record "${Label}: setup.exe /S" 'fail' "exit $($p.ExitCode), install dir '$dir'"
    }
  }
  'uninstall-nsis' {
    [void](Stop-DeckPilot)
    $dir = Get-Content $dirFile -ErrorAction SilentlyContinue
    $uninstaller = if ($dir) { Join-Path $dir 'uninstall.exe' } else { $null }
    if (-not $uninstaller -or -not (Test-Path $uninstaller)) {
      Record "${Label}: uninstall.exe /S" 'fail' "no uninstall.exe in '$dir': $(Get-ChildItem $dir -Name -ErrorAction SilentlyContinue)"
      break
    }
    $p = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
    for ($i = 0; $i -lt 60 -and (Test-Path (Join-Path $dir 'deckpilot-desktop.exe')); $i++) { Start-Sleep 1 }
    if ($p.ExitCode -eq 0 -and -not (Test-Path (Join-Path $dir 'deckpilot-desktop.exe'))) {
      Record "${Label}: uninstall.exe /S" 'pass' "exit 0; deckpilot-desktop.exe removed from $dir; entries left: $(Uninstall-Entry)"
    } else {
      Record "${Label}: uninstall.exe /S" 'fail' "exit $($p.ExitCode); files left in $dir"
    }
  }
  'launch' {
    $dir = Get-Content $dirFile -ErrorAction Stop
    $exe = Join-Path $dir 'deckpilot-desktop.exe'
    $p = New-Profile $Label
    $envs = Profile-Env $p
    $before = @(DeckPilot-Credentials)
    $realBefore = @(Real-ProfileState)
    $since = Get-Date
    $desktop = Start-WithEnv $exe $envs
    $checks = 0; $aliveDesktop = 0; $aliveAgent = 0
    for ($i = 0; $i -lt 6; $i++) {
      Start-Sleep 5
      $checks++
      if (-not $desktop.HasExited) { $aliveDesktop++ }
      if (Get-Process deckpilot-agent -ErrorAction SilentlyContinue) { $aliveAgent++ }
    }
    if ($aliveDesktop -eq $checks) { Record "${Label}: desktop alive 30 s" 'pass' "pid $($desktop.Id) alive at $aliveDesktop/$checks checks" }
    else { Record "${Label}: desktop alive 30 s" 'fail' "alive at $aliveDesktop/$checks checks; exit code $(if ($desktop.HasExited) { $desktop.ExitCode })" }
    $agents = Get-Process deckpilot-agent -ErrorAction SilentlyContinue
    if ($aliveAgent -eq $checks) {
      Record "${Label}: agent started by the app, alive 30 s" 'pass' "pids $(($agents | ForEach-Object Id) -join ' ') ($(($agents | Select-Object -First 1).Path)) at $aliveAgent/$checks checks"
    } else {
      Record "${Label}: agent started by the app, alive 30 s" 'fail' "agent seen at $aliveAgent/$checks checks"
    }
    $data = $envs.DECKPILOT_DATA_DIR
    $db = Join-Path $data 'deckpilot.db'
    for ($i = 0; $i -lt 30 -and -not (Test-Path $db); $i++) { Start-Sleep 1 }
    $listing = (Get-ChildItem -LiteralPath $data -Name -Recurse -Depth 1 -ErrorAction SilentlyContinue) -join ' '
    if (Test-Path $db) {
      $bytes = [IO.File]::ReadAllBytes($db)
      $header = [Text.Encoding]::ASCII.GetString($bytes, 0, [Math]::Min(15, $bytes.Length))
      if ($bytes.Length -gt 0 -and $header -ne 'SQLite format 3') {
        Record "${Label}: encrypted database in the isolated profile" 'pass' "$db ($($bytes.Length) bytes, no 'SQLite format 3' header); data dir: $listing"
      } else {
        Record "${Label}: encrypted database in the isolated profile" 'fail' "$db is empty or a plain SQLite file"
      }
    } else {
      $agentLog = Get-ChildItem (Join-Path $data 'logs') -File -ErrorAction SilentlyContinue | Select-Object -First 1 | Get-Content -Tail 3 -ErrorAction SilentlyContinue
      Record "${Label}: encrypted database in the isolated profile" 'fail' "no $db; data dir: $listing; agent log: $agentLog"
    }
    $shot = Screenshot "$Label-first-launch"
    if ($shot) { Record "${Label}: screenshot" 'pass' "artifact file $shot" } else { Record "${Label}: screenshot" 'fail' 'no screenshot could be taken' }
    if (Stop-DeckPilot) { Record "${Label}: stopped" 'pass' 'desktop and agent terminated' } else { Record "${Label}: stopped" 'fail' 'processes left' }
    New-Item -ItemType Directory -Force -Path (Join-Path $out "$Label-agent-logs") | Out-Null
    Copy-Item -Recurse -Force -Path (Join-Path $data 'logs\*') -Destination (Join-Path $out "$Label-agent-logs") -ErrorAction SilentlyContinue
    # Credential Manager: the system key.
    $after = @(DeckPilot-Credentials)
    $new = @($after | Where-Object { $before -notcontains $_ })
    if ($new.Count -gt 0) {
      Record "${Label}: system key in Credential Manager" 'pass' "new generic credential(s): $($new -join ', ') (Credential Manager is per user and cannot be redirected to the profile; deleted now)"
      foreach ($target in $new) { cmdkey "/delete:$target" | Out-Null }
    } else {
      Record "${Label}: system key in Credential Manager" 'fail' 'no new DeckPilot credential was created'
    }
    $realAfter = @(Real-ProfileState | Where-Object { $realBefore -notcontains $_ })
    $leaks = @(Real-ProfileLeaks $since)
    if ($realAfter.Count -eq 0 -and $leaks.Count -eq 0) {
      Record "${Label}: real profile untouched" 'pass' 'no new *deckpilot* entry in the real %APPDATA%, %LOCALAPPDATA% or %USERPROFILE%'
    } else {
      Record "${Label}: real profile untouched" 'fail' "created in the real profile: $((@($realAfter) + @($leaks) | Select-Object -Unique) -join ', ')"
    }
  }
  'gui' {
    $dir = Get-Content $dirFile -ErrorAction Stop
    $exe = Join-Path $dir 'deckpilot-desktop.exe'
    $p = New-Profile "$Label-gui"
    $envs = Profile-Env $p
    $before = @(DeckPilot-Credentials)
    $listener = [System.Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0); $listener.Start(); $port = $listener.LocalEndpoint.Port; $listener.Stop()
    $listener = [System.Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0); $listener.Start(); $native = $listener.LocalEndpoint.Port; $listener.Stop()
    $driverArgs = @('--port', "$port", '--native-port', "$native")
    if ($env:NATIVE_DRIVER) { $driverArgs += @('--native-driver', $env:NATIVE_DRIVER) }
    $log = Join-Path $out "$Label-tauri-driver.log"
    $envs['RUST_LOG'] = 'info'
    $server = Start-Process -FilePath $env:TAURI_DRIVER -ArgumentList $driverArgs -PassThru -NoNewWindow -RedirectStandardOutput $log -RedirectStandardError "$log.err" -Environment $envs
    $up = $false
    for ($i = 0; $i -lt 120 -and -not $up; $i++) {
      try { Invoke-RestMethod "http://127.0.0.1:$port/status" -TimeoutSec 2 | Out-Null; $up = $true } catch { Start-Sleep -Milliseconds 500 }
    }
    if (-not $up) {
      Record "${Label}: GUI tauri-driver" 'not-covered' "tauri-driver did not answer on port ${port}: $(Get-Content "$log.err" -Tail 5 -ErrorAction SilentlyContinue)"
    } else {
      node (Join-Path $PSScriptRoot 'gui.mjs') --port "$port" --app "$exe" --label "$Label" --out "$out" 2>&1 | Tee-Object -FilePath (Join-Path $out "$Label-gui.log")
    }
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    Get-Process msedgedriver -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    [void](Stop-DeckPilot)
    New-Item -ItemType Directory -Force -Path (Join-Path $out "$Label-gui-agent-logs") | Out-Null
    Copy-Item -Recurse -Force -Path (Join-Path $envs.DECKPILOT_DATA_DIR 'logs\*') -Destination (Join-Path $out "$Label-gui-agent-logs") -ErrorAction SilentlyContinue
    foreach ($target in @(DeckPilot-Credentials | Where-Object { $before -notcontains $_ })) { cmdkey "/delete:$target" | Out-Null }
  }
  default { Write-Error "unknown command $Command"; exit 2 }
}
exit 0
