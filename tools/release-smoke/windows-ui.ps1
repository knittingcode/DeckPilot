# Drives the INSTALLED release app from outside through Windows UI
# Automation (UIA), the accessibility layer screen readers use; dot-sourced
# by windows.ps1 (`windows.ps1 ui <label>`), which provides Record,
# Screenshot, Profile-Env and Start-WithEnv.
#
# Nothing in the app is changed or enabled for this: no WebDriver, no
# DevTools, no remote debugging. WebView2 exposes the page's accessibility
# tree (roles and aria-labels) to UIA clients like any Chromium browser,
# so elements are found by their accessible name and control type, and
# acted on through UIA patterns (Invoke, Toggle, Value, Window). Only Radix
# menu triggers and the device canvas's keys, which react to pointer
# events rather than clicks, get a real mouse click at the centre of the
# element found by name (never fixed coordinates).
#
# Steps (each recorded as "<label>: UIA <step>"; the first failure stops
# the flow and the rest are recorded as not reached):
#   main window › toolbar (Marketplace, Settings, device and profile
#   selectors, the rail) › agent and its tray icon › Marketplace lists the
#   production catalog › install Memory with consent › the settings window
#   (its own top-level window) lists it › enable with consent › uninstall ›
#   a profile created in Settings › Device Editor with a model, the Action
#   Library and the key inspector › closing the main window (the app ends,
#   the agent keeps running) › reopening restores the window and the
#   profile being edited.

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
Add-Type -Namespace SmokeUi -Name Native -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, System.IntPtr extra);
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr hwnd);
[DllImport("user32.dll")] public static extern bool ShowWindow(System.IntPtr hwnd, int cmd);
'@

$script:AE = [System.Windows.Automation.AutomationElement]
$script:CT = [System.Windows.Automation.ControlType]
$script:Scope = [System.Windows.Automation.TreeScope]
$script:UiWait = 30
$script:UiLong = 120
$script:UiShots = 0
$script:PluginName = 'Memory'
$script:PluginVersion = '0.1.0'
$script:ProfileName = 'Release smoke'

function Ui-Prop($Property, $Value) { [System.Windows.Automation.PropertyCondition]::new($Property, $Value) }

function Ui-Name($e) { try { [string]$e.Current.Name } catch { '' } }
function Ui-Type($e) { try { $e.Current.ControlType.ProgrammaticName -replace '^ControlType\.', '' } catch { '?' } }

# Top-level windows of a process (the WebView2 content belongs to them).
function Ui-TopWindows([int]$ProcessId) {
  $found = $script:AE::RootElement.FindAll($script:Scope::Children, (Ui-Prop $script:AE::ProcessIdProperty $ProcessId))
  return @($found | Where-Object { $_ })
}

function Ui-Window([int]$ProcessId, [string]$Name) {
  return Ui-TopWindows $ProcessId | Where-Object { (Ui-Name $_) -eq $Name } | Select-Object -First 1
}

# Every descendant of $Root matching $Test (a scriptblock over $_).
function Ui-All($Root, [scriptblock]$Test) {
  try {
    $all = $Root.FindAll($script:Scope::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
  } catch { return @() }
  return @($all | Where-Object { $_ } | Where-Object $Test)
}

function Ui-Until([string]$What, [scriptblock]$Probe, [int]$Seconds = $script:UiWait) {
  $deadline = (Get-Date).AddSeconds($Seconds)
  $last = $null
  while ($true) {
    try {
      $value = & $Probe
      if ($value) { return $value }
    } catch { $last = $_ }
    if ((Get-Date) -gt $deadline) {
      throw "timed out after $Seconds s waiting for $What$(if ($last) { " (last error: $last)" })"
    }
    Start-Sleep -Milliseconds 500
  }
}

function Ui-Find($Root, [string]$What, [scriptblock]$Test, [int]$Seconds = $script:UiWait) {
  return Ui-Until $What { Ui-All $Root $Test | Select-Object -First 1 } $Seconds
}

function Ui-Button($Root, [string]$Name, [int]$Seconds = $script:UiWait) {
  return Ui-Find $Root "button '$Name'" { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq $Name } $Seconds
}

function Ui-Pattern($e, $Pattern) {
  $p = $null
  if ($e.TryGetCurrentPattern($Pattern, [ref]$p)) { return $p }
  return $null
}

function Ui-Invoke($e) {
  $p = Ui-Pattern $e ([System.Windows.Automation.InvokePattern]::Pattern)
  if ($p) { $p.Invoke(); return 'Invoke' }
  $p = Ui-Pattern $e ([System.Windows.Automation.TogglePattern]::Pattern)
  if ($p) { $p.Toggle(); return 'Toggle' }
  $p = Ui-Pattern $e ([System.Windows.Automation.SelectionItemPattern]::Pattern)
  if ($p) { $p.Select(); return 'SelectionItem' }
  throw "'$(Ui-Name $e)' ($(Ui-Type $e)) supports neither Invoke, Toggle nor SelectionItem"
}

function Ui-Focus($Window) {
  try { $Window.SetFocus() } catch {}
  $hwnd = [IntPtr]$Window.Current.NativeWindowHandle
  if ($hwnd -ne [IntPtr]::Zero) { [void][SmokeUi.Native]::SetForegroundWindow($hwnd) }
  Start-Sleep -Milliseconds 300
}

# A real left click at the centre of the element (found by name).
function Ui-Click($Window, $e) {
  Ui-Focus $Window
  $r = $e.Current.BoundingRectangle
  if ($r.IsEmpty -or $r.Width -le 0) { throw "'$(Ui-Name $e)' has no bounding rectangle (off screen?)" }
  $x = [int]($r.X + $r.Width / 2); $y = [int]($r.Y + $r.Height / 2)
  [void][SmokeUi.Native]::SetCursorPos($x, $y)
  Start-Sleep -Milliseconds 100
  [SmokeUi.Native]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 80
  [SmokeUi.Native]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
  Start-Sleep -Milliseconds 300
  return "click at ($x,$y)"
}

function Ui-Shot([string]$Name) {
  $script:UiShots++
  $file = Screenshot ("{0}-ui-{1:D2}-{2}" -f $Label, $script:UiShots, $Name)
  if ($file) { return $file }
  return "(screenshot $Name failed)"
}

# Names of the accessible elements of a window, for the failure report.
function Ui-Dump($Root, [string]$File) {
  try {
    $lines = Ui-All $Root { $true } | ForEach-Object {
      $r = $_.Current.BoundingRectangle
      "{0}`t{1}`t{2}`t{3}" -f (Ui-Type $_), (Ui-Name $_), $_.Current.AutomationId, $(if ($r.IsEmpty) { '' } else { "$([int]$r.X),$([int]$r.Y) $([int]$r.Width)x$([int]$r.Height)" })
    }
    Set-Content -Path (Join-Path $out $File) -Value $lines -Encoding utf8
    return "$File ($($lines.Count) elements)"
  } catch { return "(dump failed: $_)" }
}

function Ui-Texts($Root) {
  return (Ui-All $Root { (Ui-Name $_) -ne '' } | ForEach-Object { Ui-Name $_ })
}

function Ui-HasText($Root, [string]$Text) {
  return [bool](Ui-All $Root { (Ui-Name $_) -eq $Text } | Select-Object -First 1)
}

function Ui-Rect($Window) {
  $r = $Window.Current.BoundingRectangle
  return "$([int]$r.X),$([int]$r.Y) $([int]$r.Width)x$([int]$r.Height)"
}

function Ui-Desktop {
  return Get-Process deckpilot-desktop -ErrorAction SilentlyContinue | Where-Object { -not $_.HasExited } | Select-Object -First 1
}

function Ui-MainWindow([int]$Seconds = $script:UiLong) {
  return Ui-Until 'the main window (DeckPilot) with its toolbar' {
    $d = Ui-Desktop
    if (-not $d) { return $null }
    $w = Ui-Window $d.Id 'DeckPilot'
    if ($w -and (Ui-All $w { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Marketplace' } | Select-Object -First 1)) { return $w }
    return $null
  } $Seconds
}

function Ui-SettingsWindow([int]$Seconds = $script:UiLong) {
  return Ui-Until "the settings window (DeckPilot Settings)" {
    $d = Ui-Desktop
    if ($d) { Ui-Window $d.Id 'DeckPilot Settings' }
  } $Seconds
}

# The Marketplace at its tabs (a plugin's details replace them).
function Ui-Marketplace($Main) {
  $back = Ui-All $Main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Back to plugins' } | Select-Object -First 1
  if ($back) { [void](Ui-Invoke $back) }
  if (-not (Ui-All $Main { (Ui-Type $_) -eq 'TabItem' -and (Ui-Name $_) -like 'Discover*' } | Select-Object -First 1)) {
    [void](Ui-Invoke (Ui-Button $Main 'Marketplace'))
  }
  return Ui-Find $Main "the Marketplace's tabs" { (Ui-Type $_) -eq 'TabItem' -and (Ui-Name $_) -like 'Discover*' }
}

# Radix tabs switch on focus or mouse down, not on a click event.
function Ui-Tab($Main, [string]$Name) {
  $tab = Ui-Find $Main "tab '$Name'" { (Ui-Type $_) -eq 'TabItem' -and (Ui-Name $_) -like "$Name*" }
  try { $tab.SetFocus() } catch {}
  Start-Sleep -Milliseconds 500
  $p = Ui-Pattern $tab ([System.Windows.Automation.SelectionItemPattern]::Pattern)
  if ($p -and -not $p.Current.IsSelected) { [void](Ui-Click $Main $tab) }
  return $tab
}

function Ui-CheckAll($Root) {
  $n = 0
  foreach ($box in (Ui-All $Root { (Ui-Type $_) -eq 'CheckBox' })) {
    $t = Ui-Pattern $box ([System.Windows.Automation.TogglePattern]::Pattern)
    if ($t -and $t.Current.ToggleState -ne [System.Windows.Automation.ToggleState]::On) {
      $t.Toggle(); $n++
      Start-Sleep -Milliseconds 200
    }
  }
  return $n
}

function Ui-PluginCard($Main) {
  return Ui-All $Main { (Ui-Type $_) -eq 'Button' -and ((Ui-Name $_) -eq $script:PluginName -or (Ui-Name $_) -like "$($script:PluginName) *") } | Select-Object -First 1
}

# The production catalog the agent fetched into the isolated profile.
function Ui-CatalogNames([string]$DataDir) {
  $file = Join-Path $DataDir 'plugin-registry\registry-index.json'
  if (-not (Test-Path $file)) { return @() }
  try {
    $index = Get-Content -Raw $file | ConvertFrom-Json
    $plugins = if ($index.plugins) { $index.plugins } elseif ($index.signed.plugins) { $index.signed.plugins } else { @() }
    return @($plugins | ForEach-Object { if ($_.name) { $_.name } elseif ($_.latest.name) { $_.latest.name } } | Where-Object { $_ } | Sort-Object -Unique)
  } catch { return @() }
}

function Invoke-UiFlow([string]$Exe, $Envs) {
  $script:Exe = $Exe
  $script:Envs = $Envs
  $state = @{}
  $steps = [ordered]@{
    'main window' = {
      $script:Desktop = Start-WithEnv $script:Exe $script:Envs
      $main = Ui-MainWindow
      $state.main = $main
      $state.rect = Ui-Rect $main
      $web = Ui-All $main { $_.Current.FrameworkId -eq 'Chrome' } | Select-Object -First 1
      "top-level window 'DeckPilot' of deckpilot-desktop pid $($script:Desktop.Id) (class $($main.Current.ClassName), $($state.rect)); web content exposed through UIA (framework '$(if ($web) { $web.Current.FrameworkId } else { '?' })'); $(Ui-Shot 'main')"
    }
    'toolbar' = {
      $main = $state.main
      $names = @(Ui-All $main { (Ui-Type $_) -eq 'Button' } | ForEach-Object { Ui-Name $_ } | Where-Object { $_ })
      $want = [ordered]@{
        'Marketplace' = { $_ -eq 'Marketplace' }
        'Settings' = { $_ -like 'Settings*' }
        'device selector' = { $_ -like 'Device:*' -or $_ -eq 'Choose a device' }
        'profile selector' = { $_ -like 'Profile:*' -or $_ -eq 'Choose the profile to edit' }
      }
      $missing = @(); $seen = @()
      foreach ($w in $want.GetEnumerator()) {
        $hit = $names | Where-Object $w.Value | Select-Object -First 1
        if ($hit) { $seen += "$($w.Key) '$hit'" } else { $missing += $w.Key }
      }
      if ($missing) { throw "missing in the toolbar: $($missing -join ', '); buttons: $($names -join ' | ')" }
      $title = if (Ui-HasText $main 'Device Editor') { "page title 'Device Editor'" } else { 'no page title' }
      $rail = @($names | Where-Object { $_ -in 'Device Editor', 'Dashboard', 'Action Library', 'Workflows' })
      "$($seen -join '; '); $title; rail: $(if ($rail) { $rail -join ', ' } else { 'labels not exposed' }); all buttons: $($names -join ' | ')"
    }
    'agent and tray icon' = {
      $agent = Ui-Until 'deckpilot-agent' { Get-Process deckpilot-agent -ErrorAction SilentlyContinue | Select-Object -First 1 }
      $tray = $null
      $taskbar = $script:AE::RootElement.FindFirst($script:Scope::Children, (Ui-Prop $script:AE::ClassNameProperty 'Shell_TrayWnd'))
      if ($taskbar) { $tray = Ui-All $taskbar { (Ui-Name $_) -like 'DeckPilot*' } | Select-Object -First 1 }
      $where = 'taskbar'
      if (-not $tray -and $taskbar) {
        # Hidden icons: the notification area's overflow.
        $chevron = Ui-All $taskbar { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -match 'hidden icons' } | Select-Object -First 1
        if ($chevron) {
          [void](Ui-Invoke $chevron)
          Start-Sleep 2
          foreach ($w in $script:AE::RootElement.FindAll($script:Scope::Children, [System.Windows.Automation.Condition]::TrueCondition)) {
            $tray = Ui-All $w { (Ui-Name $_) -like 'DeckPilot*' -and (Ui-Type $_) -eq 'Button' } | Select-Object -First 1
            if ($tray) { $where = "overflow ($($w.Current.ClassName))"; break }
          }
          $state.trayShot = Ui-Shot 'tray-overflow'
          [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
        }
      }
      $state.trayEvidence = if ($tray) { "tray icon '$(Ui-Name $tray)' ($(Ui-Type $tray)) in the $where" } else { 'no notification-area element named DeckPilot* found' }
      "deckpilot-agent pid $($agent.Id) ($($agent.Path)); $($state.trayEvidence)$(if ($state.trayShot) { "; $($state.trayShot)" })"
    }
    'Marketplace lists the production catalog' = {
      $main = $state.main
      [void](Ui-Marketplace $main)
      [void](Ui-Tab $main 'Discover')
      $card = Ui-Until "the '$($script:PluginName)' card" { Ui-PluginCard $main } $script:UiLong
      $catalog = @(Ui-CatalogNames $script:Envs.DECKPILOT_DATA_DIR)
      $texts = @(Ui-Texts $main)
      $shown = @($catalog | Where-Object { $n = $_; $texts | Where-Object { $_ -eq $n -or $_ -like "$n *" } | Select-Object -First 1 })
      $absent = @($catalog | Where-Object { $shown -notcontains $_ })
      if ($catalog.Count -gt 0 -and $absent.Count -gt 0) { throw "catalog plugins not shown: $($absent -join ', ') (shown: $($shown -join ', '))" }
      "card '$(Ui-Name $card)'; production catalog (registry-index.json in the profile): $(if ($catalog) { $catalog -join ', ' } else { 'not readable' }); all shown; $(Ui-Shot 'marketplace')"
    }
    "install $($script:PluginName) with consent" = {
      $main = $state.main
      [void](Ui-Invoke (Ui-PluginCard $main))
      [void](Ui-Invoke (Ui-Button $main "Install $($script:PluginVersion)"))
      $confirm = Ui-Button $main 'Install'
      $consentShot = Ui-Shot 'install-consent'
      $boxes = Ui-CheckAll $main
      [void](Ui-Invoke $confirm)
      [void](Ui-Until 'the consent dialog to close' { -not (Ui-All $main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Install' } | Select-Object -First 1) })
      $status = Ui-Until "'Installed $($script:PluginVersion)'" { Ui-All $main { (Ui-Name $_) -like "Installed $($script:PluginVersion)*" } | Select-Object -First 1 } $script:UiLong
      "$boxes consent box(es) checked; status '$(Ui-Name $status)'; $consentShot; $(Ui-Shot 'installed')"
    }
    'Settings opens its own window, Plugins lists it' = {
      $main = $state.main
      $settingsButton = Ui-Find $main 'the Settings button' { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -like 'Settings*' }
      [void](Ui-Invoke $settingsButton)
      $settings = Ui-SettingsWindow
      $state.settingsHwnd = $settings.Current.NativeWindowHandle
      [void](Ui-Button $settings 'Back to DeckPilot')
      [void](Ui-Invoke (Ui-Button $settings 'Plugins'))
      $row = Ui-Until "$($script:PluginName) v$($script:PluginVersion) in the plugin list" {
        (Ui-HasText $settings $script:PluginName) -and (Ui-HasText $settings "v$($script:PluginVersion)")
      }
      $shot = Ui-Shot 'settings-plugins'
      $w = Ui-Pattern $settings ([System.Windows.Automation.WindowPattern]::Pattern)
      if ($w) { $w.Close() } else { [void](Ui-Invoke (Ui-Button $settings 'Back to DeckPilot')) }
      [void](Ui-Until 'the settings window to close' { $d = Ui-Desktop; -not ($d -and (Ui-Window $d.Id 'DeckPilot Settings')) })
      "separate top-level window 'DeckPilot Settings' (hwnd $($state.settingsHwnd), main window hwnd $($main.Current.NativeWindowHandle)); Extensions > Plugins lists $($script:PluginName) v$($script:PluginVersion); closed; $shot"
    }
    'enable with consent' = {
      $main = $state.main
      [void](Ui-Marketplace $main)
      [void](Ui-Tab $main 'Installed')
      $switch = Ui-Find $main "switch 'Enable $($script:PluginName)'" { (Ui-Name $_) -eq "Enable $($script:PluginName)" }
      [void](Ui-Invoke $switch)
      $confirm = Ui-Button $main 'Enable'
      $consentShot = Ui-Shot 'enable-consent'
      $boxes = Ui-CheckAll $main
      [void](Ui-Invoke $confirm)
      $on = Ui-Find $main "switch 'Disable $($script:PluginName)'" { (Ui-Name $_) -eq "Disable $($script:PluginName)" } $script:UiLong
      "$boxes consent box(es); switch now '$(Ui-Name $on)'; $consentShot; $(Ui-Shot 'enabled')"
    }
    'plugin toolbar entries' = {
      # No production plugin contributes toolbar entries (contributes.navigation).
      $nav = Ui-All $state.main { (Ui-Name $_) -eq 'Plugins' -and (Ui-Type $_) -ne 'Button' -and (Ui-Type $_) -ne 'List' } | Select-Object -First 1
      $state.navEvidence = if ($nav) { "a 'Plugins' navigation is shown ($(Ui-Type $nav))" } else { "no 'Plugins' navigation in the toolbar" }
      'skip'
    }
    'uninstall' = {
      $main = $state.main
      [void](Ui-Marketplace $main)
      [void](Ui-Tab $main 'Installed')
      $buttons = Ui-Until "an Uninstall button" { @(Ui-All $main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Uninstall' }) }
      # The installed list's own Uninstall of the plugin (the bundled plugins have none or their own).
      $item = Ui-Find $main "the installed list" { (Ui-Name $_) -eq 'Installed plugins' }
      $mine = Ui-All $item { (Ui-Type $_) -eq 'ListItem' -and (Ui-HasText $_ $script:PluginName) } | Select-Object -First 1
      $button = if ($mine) { Ui-All $mine { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Uninstall' } | Select-Object -First 1 } else { $null }
      if (-not $button) { throw "no Uninstall button in the '$($script:PluginName)' item of the installed list ($(@($buttons).Count) Uninstall buttons in the window)" }
      [void](Ui-Invoke $button)
      $confirmShot = Ui-Shot 'uninstall-confirm'
      # The confirmation dialog's button comes after the list's in the tree.
      $dialogButton = Ui-Until 'the confirmation' {
        $all = @(Ui-All $main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Uninstall' })
        if ($all.Count -gt @($buttons).Count) { $all[-1] }
      }
      [void](Ui-Invoke $dialogButton)
      [void](Ui-Until "$($script:PluginName) to leave the installed list" {
        $list = Ui-All $main { (Ui-Name $_) -eq 'Installed plugins' } | Select-Object -First 1
        $list -and -not (Ui-All $list { (Ui-Name $_) -eq $script:PluginName } | Select-Object -First 1)
      } $script:UiLong)
      "$($script:PluginName) gone from Marketplace > Installed; $confirmShot; $(Ui-Shot 'uninstalled')"
    }
    'profile created in Settings' = {
      $main = $state.main
      $create = Ui-All $main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Create a profile' } | Select-Object -First 1
      if (-not $create) {
        # The Marketplace is still open: back to the Device Editor (rail).
        $editor = Ui-All $main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Device Editor' } | Select-Object -First 1
        if ($editor) { [void](Ui-Invoke $editor) }
        $create = Ui-Button $main 'Create a profile'
      }
      [void](Ui-Invoke $create)
      $settings = Ui-SettingsWindow
      [void](Ui-Invoke (Ui-Button $settings 'New profile'))
      $field = Ui-Find $settings "the profile's Name field" { (Ui-Type $_) -eq 'Edit' -and (Ui-Name $_) -eq 'Name' }
      $value = Ui-Pattern $field ([System.Windows.Automation.ValuePattern]::Pattern)
      $value.SetValue($script:ProfileName)
      Start-Sleep -Milliseconds 500
      $save = Ui-Button $settings 'Save'
      if (-not $save.Current.IsEnabled) {
        # The page did not see the value: type it.
        $field.SetFocus(); [System.Windows.Forms.SendKeys]::SendWait($script:ProfileName); Start-Sleep -Milliseconds 500
      }
      [void](Ui-Invoke $save)
      [void](Ui-Until "'$($script:ProfileName)' in All profiles" { -not (Ui-All $settings { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Save' } | Select-Object -First 1) -and (Ui-HasText $settings $script:ProfileName) })
      $shot = Ui-Shot 'settings-profiles'
      $w = Ui-Pattern $settings ([System.Windows.Automation.WindowPattern]::Pattern)
      if ($w) { $w.Close() }
      "Create a profile opened Settings > Profiles; New profile '$($script:ProfileName)' saved and listed; $shot"
    }
    'Device Editor, Action Library and key inspector' = {
      $main = $state.main
      $profile = Ui-Until "the profile selector on '$($script:ProfileName)'" { Ui-All $main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -like 'Profile:*' -or (Ui-Name $_) -eq 'Choose the profile to edit' } | Select-Object -First 1 }
      if ((Ui-Name $profile) -notlike "Profile: $($script:ProfileName)*") {
        [void](Ui-Click $main $profile)
        [void](Ui-Invoke (Ui-Find $main "menu item '$($script:ProfileName)'" { (Ui-Type $_) -eq 'MenuItem' -and (Ui-Name $_) -like "*$($script:ProfileName)*" }))
        $profile = Ui-Find $main "the profile selector on '$($script:ProfileName)'" { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -like "Profile: $($script:ProfileName)*" }
      }
      $device = Ui-Find $main 'the device selector' { (Ui-Type $_) -eq 'Button' -and ((Ui-Name $_) -like 'Device:*' -or (Ui-Name $_) -eq 'Choose a device') -and $_.Current.IsEnabled } $script:UiLong
      [void](Ui-Click $main $device)
      $models = Ui-Until 'the device menu' { @(Ui-All $main { (Ui-Type $_) -eq 'MenuItem' -and $_.Current.IsEnabled }) }
      $model = $models | Select-Object -First 1
      $menuShot = Ui-Shot 'device-menu'
      [void](Ui-Invoke $model)
      $canvas = Ui-Find $main 'the device canvas' { (Ui-Name $_) -like '* controls' -and (Ui-Type $_) -ne 'Button' } $script:UiLong
      $library = Ui-Find $main 'the Action Library' { (Ui-Name $_) -eq 'Action Library' -and (Ui-Type $_) -ne 'Button' }
      [void](Ui-Find $main 'the inspector' { (Ui-Name $_) -like 'Select a key, dial or touch zone*' })
      $key = Ui-Find $main "'Key 1'" { (Ui-Name $_) -eq 'Key 1' -and (Ui-Type $_) -ne 'Text' }
      $how = Ui-Click $main $key
      [void](Ui-Until "the inspector of Key 1" { -not (Ui-All $main { (Ui-Name $_) -like 'Select a key, dial or touch zone*' } | Select-Object -First 1) })
      $device = Ui-Find $main 'the device selector' { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -like 'Device:*' }
      $state.editorRect = Ui-Rect $main
      "profile selector '$(Ui-Name $profile)'; device menu: $(($models | ForEach-Object { Ui-Name $_ }) -join ' | '); chose '$(Ui-Name $model)' -> '$(Ui-Name $device)'; canvas '$(Ui-Name $canvas)'; library '$(Ui-Name $library)' ($(Ui-Type $library)); Key 1 selected ($how), the inspector shows it; $menuShot; $(Ui-Shot 'editor')"
    }
    'closing the main window' = {
      $main = $state.main
      $w = Ui-Pattern $main ([System.Windows.Automation.WindowPattern]::Pattern)
      $pidBefore = $script:Desktop.Id
      $w.Close()
      [void](Ui-Until 'deckpilot-desktop to end' { -not (Get-Process -Id $pidBefore -ErrorAction SilentlyContinue) })
      Start-Sleep 3
      $agent = Get-Process deckpilot-agent -ErrorAction SilentlyContinue | Select-Object -First 1
      if (-not $agent) { throw 'the agent ended with the window' }
      "WindowPattern.Close: deckpilot-desktop pid $pidBefore ended with its last window; deckpilot-agent pid $($agent.Id) keeps running; $(Ui-Shot 'closed')"
    }
    'reopening restores the window and the profile' = {
      $script:Desktop = Start-WithEnv $script:Exe $script:Envs
      $main = Ui-MainWindow
      $state.main = $main
      $rect = Ui-Rect $main
      $profile = Ui-Find $main "the profile selector" { (Ui-Type $_) -eq 'Button' -and ((Ui-Name $_) -like 'Profile:*' -or (Ui-Name $_) -eq 'Choose the profile to edit') }
      $device = Ui-Find $main "the device selector" { (Ui-Type $_) -eq 'Button' -and ((Ui-Name $_) -like 'Device:*' -or (Ui-Name $_) -eq 'Choose a device') }
      if ((Ui-Name $profile) -notlike "Profile: $($script:ProfileName)*") { throw "after reopening the profile selector says '$(Ui-Name $profile)'" }
      $same = if ($rect -eq $state.rect) { 'same as at first launch' } else { "first launch $($state.rect)" }
      "started again: window 'DeckPilot' at $rect ($same); profile selector '$(Ui-Name $profile)'; device selector '$(Ui-Name $device)'; $(Ui-Shot 'reopened')"
    }
  }
  $failed = $null
  foreach ($name in $steps.Keys) {
    $step = "${Label}: UIA $name"
    if ($failed) { Record $step 'not-covered' "not reached: '$failed' failed"; continue }
    try {
      $evidence = & $steps[$name]
      if ($name -eq 'plugin toolbar entries') {
        Record $step 'not-covered' "no production plugin contributes toolbar entries (contributes.navigation) to show; with $($script:PluginName) enabled: $($state.navEvidence)"
      } else {
        Record $step 'pass' $evidence
      }
    } catch {
      $failed = $name
      $shot = Ui-Shot ("failure-" + ($name -replace '[^A-Za-z0-9]+', '-'))
      $dump = 'no window to dump'
      $d = Ui-Desktop
      if ($d) {
        $i = 0
        foreach ($w in (Ui-TopWindows $d.Id)) { $i++; $dump = Ui-Dump $w "$Label-ui-failure-tree-$i.tsv" }
      }
      Record $step 'fail' "$($_.Exception.Message); $shot; UIA tree: $dump"
    }
  }
}
