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
# the flow and the rest are recorded as not reached, except after a failed
# settings window: the app is restarted and the steps that do not need
# that window still run):
#   main window › toolbar (Marketplace, Settings, device and profile
#   selectors, the rail) › agent and its tray icon › Marketplace lists the
#   production catalog › install Memory with consent › Settings › Plugins
#   lists it › enable with consent › uninstall › a profile created in
#   Settings › Device Editor with a model, the Action Library and the key
#   inspector › closing the main window (the app ends, the agent keeps
#   running) › reopening restores the window and the profile being edited.
#
# Two shells are accepted, told apart by what is on screen, and each step
# says which one it saw: up to 0.8.1 the settings are a top-level window of
# their own ("DeckPilot Settings", "Back to DeckPilot" returns) and the
# toolbar has Back/Forward and the page title; later releases show the
# settings as a view of the main window (a sidebar labelled "Settings" with
# "Search settings"), opened without any window and left with the rail or
# the toolbar, and the toolbar has neither.
#
# A failed step also lists every top-level window of the app, the hidden
# ones too (EnumWindows), and whether each window's thread still answers
# messages (IsHungAppWindow, WM_NULL): <label>-ui-windows.txt.

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms, System.Drawing
Add-Type -Namespace SmokeUi -Name Native -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, System.IntPtr extra);
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr hwnd);
[DllImport("user32.dll")] public static extern bool ShowWindow(System.IntPtr hwnd, int cmd);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr hwnd);
[DllImport("user32.dll")] public static extern bool IsHungAppWindow(System.IntPtr hwnd);
[DllImport("user32.dll")] public static extern System.IntPtr SendMessageTimeout(System.IntPtr hwnd, uint msg, System.IntPtr w, System.IntPtr l, uint flags, uint timeout, out System.IntPtr result);
[DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, System.IntPtr param);
[DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(System.IntPtr hwnd, out uint pid);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(System.IntPtr hwnd, System.Text.StringBuilder text, int max);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(System.IntPtr hwnd, System.Text.StringBuilder text, int max);
delegate bool EnumProc(System.IntPtr hwnd, System.IntPtr param);
// Every top-level window of a process, visible or not (UI Automation lists
// only the visible ones): "hwnd|class|title|visible|thread".
public static string[] TopLevel(uint processId) {
  var found = new System.Collections.Generic.List<string>();
  EnumWindows((hwnd, param) => {
    uint owner;
    uint thread = GetWindowThreadProcessId(hwnd, out owner);
    if (owner == processId) {
      var title = new System.Text.StringBuilder(256);
      var cls = new System.Text.StringBuilder(256);
      GetWindowText(hwnd, title, 256);
      GetClassName(hwnd, cls, 256);
      found.Add(hwnd.ToInt64() + "|" + cls + "|" + title + "|" + IsWindowVisible(hwnd) + "|" + thread);
    }
    return true;
  }, System.IntPtr.Zero);
  return found.ToArray();
}
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
$script:LayoutName = 'Release smoke layout'

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
  # Chromium gives a button that opens a popup (aria-haspopup, e.g. a Radix
  # alert dialog's trigger) ExpandCollapse instead of Invoke (run
  # 44541282680): those triggers react to a click, so they get a real one.
  if (Ui-Pattern $e ([System.Windows.Automation.ExpandCollapsePattern]::Pattern)) {
    return Ui-Click (Ui-TopLevelOf $e) $e
  }
  throw "'$(Ui-Name $e)' ($(Ui-Type $e)) supports neither Invoke, Toggle, SelectionItem nor ExpandCollapse"
}

# The top-level window an element belongs to.
function Ui-TopLevelOf($e) {
  $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
  $root = $script:AE::RootElement
  $current = $e
  while ($true) {
    $parent = $walker.GetParent($current)
    if (-not $parent -or $parent -eq $root) { return $current }
    $current = $parent
  }
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

# The text of an element and its descendants in tree order: Chromium
# splits text into runs ("Installed " + "0.1.0").
function Ui-Joined($Root) {
  $names = @((Ui-Name $Root)) + @(Ui-All $Root { $true } | ForEach-Object { Ui-Name $_ })
  return (($names | Where-Object { $_ }) -join '') -replace '\s+', ' '
}

function Ui-HasText($Root, [string]$Text) {
  if (Ui-All $Root { (Ui-Name $_) -eq $Text } | Select-Object -First 1) { return $true }
  if ((Ui-Joined $Root).Contains($Text)) { return $true }
  # FindAll searches the control view: Chromium leaves the plain text runs
  # of a list item named by aria-labelledby out of it (the installed list's
  # "v0.1.0", run 44665060136), the raw view has them.
  return (Ui-RawJoined $Root).Contains($Text)
}

# The names of an element and its descendants in the raw view, in tree
# order, joined like Ui-Joined; at most $Max elements.
function Ui-RawJoined($Root, [int]$Max = 5000) {
  $walker = [System.Windows.Automation.TreeWalker]::RawViewWalker
  $names = New-Object System.Collections.Generic.List[string]
  $stack = New-Object System.Collections.Generic.Stack[object]
  $stack.Push($Root)
  $seen = 0
  try {
    while ($stack.Count -gt 0 -and $seen -lt $Max) {
      $e = $stack.Pop()
      $seen++
      $n = Ui-Name $e
      if ($n) { $names.Add($n) }
      $children = New-Object System.Collections.Generic.List[object]
      $c = $walker.GetFirstChild($e)
      while ($c) { $children.Add($c); $c = $walker.GetNextSibling($c) }
      for ($i = $children.Count - 1; $i -ge 0; $i--) { $stack.Push($children[$i]) }
    }
  } catch { }
  return ($names -join '') -replace '\s+', ' '
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

# The settings as a view of the main window (releases after 0.8.1): their
# search field, unique to the settings' sidebar, described with the sidebar
# labelled "Settings" around it; $null when they are not shown.
function Ui-SettingsView($Main) {
  $search = Ui-All $Main { (Ui-Type $_) -eq 'Edit' -and (Ui-Name $_) -eq 'Search settings' } | Select-Object -First 1
  if (-not $search) { return $null }
  $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
  $e = $search
  for ($i = 0; $i -lt 8; $i++) {
    try { $e = $walker.GetParent($e) } catch { $e = $null }
    if (-not $e -or $e -eq $script:AE::RootElement) { break }
    if ((Ui-Name $e) -eq 'Settings' -and (Ui-Type $e) -ne 'Button') { return "sidebar 'Settings' ($(Ui-Type $e)) with 'Search settings'" }
  }
  return "'Search settings' (the sidebar's own label not exposed)"
}

# After a command that opens the settings: 'window' (up to 0.8.1, the
# visible top-level window) or 'view' (in the main window); $null on time-out.
function Ui-SettingsAppeared($Main, [int]$Seconds = $script:UiLong) {
  try {
    return Ui-Until "the settings: a window 'DeckPilot Settings' (up to 0.8.1) or the settings view in the main window" {
      $d = Ui-Desktop
      if ($d -and (Ui-Window $d.Id 'DeckPilot Settings')) { return 'window' }
      if (Ui-SettingsView $Main) { return 'view' }
    } $Seconds
  } catch { return $null }
}

# The toolbar's Settings button: a toggle (aria-pressed), unlike the
# settings' breadcrumb "Settings".
function Ui-Gear($Main) {
  $all = @(Ui-All $Main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -like 'Settings*' })
  $toggle = $all | Where-Object { Ui-Pattern $_ ([System.Windows.Automation.TogglePattern]::Pattern) } | Select-Object -First 1
  if ($toggle) { return $toggle }
  return $all | Select-Object -First 1
}

function Ui-Pressed($e) {
  if (-not $e) { return 'not found' }
  $t = Ui-Pattern $e ([System.Windows.Automation.TogglePattern]::Pattern)
  if (-not $t) { return 'pressed state not exposed (no Toggle pattern)' }
  if ($t.Current.ToggleState -eq [System.Windows.Automation.ToggleState]::On) { return 'pressed' }
  return 'not pressed'
}

# Every titled top-level window of the app, hidden ones too (EnumWindows),
# but not the input method's own ("hwnd|class|title|visible|thread").
function Ui-AllWindows {
  $d = Ui-Desktop
  if (-not $d) { return @() }
  return @([SmokeUi.Native]::TopLevel([uint32]$d.Id) | Where-Object {
    $f = $_ -split '\|', 5
    $f[2] -ne '' -and $f[1] -notin 'IME', 'MSCTFIME UI'
  })
}

# Asserts that opening the settings view created no window, not even a
# hidden one (0.8.0 and 0.8.1 created a settings window that never showed).
function Ui-NoNewWindow($Main, [int]$VisibleBefore, [int]$TitledBefore) {
  Start-Sleep 2
  $d = Ui-Desktop
  if (-not $d) { throw 'deckpilot-desktop ended' }
  $all = Ui-AllWindows
  $settings = @($all | Where-Object { ($_ -split '\|', 5)[2] -eq 'DeckPilot Settings' })
  if ($settings) { throw "the settings view is in the main window, but a window 'DeckPilot Settings' exists too (visible $((($settings[0]) -split '\|', 5)[3]))" }
  $visible = @(Ui-TopWindows $d.Id).Count
  $titled = @($all).Count
  if ($visible -gt $VisibleBefore -or $titled -gt $TitledBefore) { throw "the settings view is in the main window, but windows appeared: visible top-level $VisibleBefore -> $visible, titled (hidden too) $TitledBefore -> $titled" }
  if (Ui-All $Main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Back to DeckPilot' } | Select-Object -First 1) { throw "the settings view has a 'Back to DeckPilot' button" }
  return "no other window (visible top-level $VisibleBefore -> $visible, titled incl. hidden $TitledBefore -> $titled), no 'Back to DeckPilot'"
}

function Ui-WindowCounts {
  $d = Ui-Desktop
  if (-not $d) { return @(0, 0) }
  return @(@(Ui-TopWindows $d.Id).Count, @(Ui-AllWindows).Count)
}

# Whether the thread of a window pumps its messages: IsHungAppWindow (no
# message retrieved for 5 s) and a WM_NULL round trip within 5 s.
function Ui-Responds([IntPtr]$Hwnd) {
  $result = [IntPtr]::Zero
  # SMTO_ABORTIFHUNG (0x2)
  $answered = [SmokeUi.Native]::SendMessageTimeout($Hwnd, 0, [IntPtr]::Zero, [IntPtr]::Zero, 0x2, 5000, [ref]$result) -ne [IntPtr]::Zero
  return "IsHungAppWindow=$([SmokeUi.Native]::IsHungAppWindow($Hwnd)), WM_NULL answered within 5 s=$answered"
}

# All top-level windows of the app (also the hidden ones UIA does not list)
# and whether their threads respond; written to <label>-ui-windows.txt.
function Ui-WindowReport {
  $d = Ui-Desktop
  if (-not $d) { return 'deckpilot-desktop is not running' }
  $lines = @([SmokeUi.Native]::TopLevel([uint32]$d.Id) | ForEach-Object {
    $f = $_ -split '\|', 5
    "hwnd $($f[0]) class '$($f[1])' title '$($f[2])' visible $($f[3]) thread $($f[4]); $(Ui-Responds ([IntPtr][long]$f[0]))"
  })
  $file = "$Label-ui-windows.txt"
  Set-Content -Path (Join-Path $out $file) -Value $lines -Encoding utf8
  $titled = @($lines | Where-Object { $_ -notmatch "title ''" })
  return "$file ($($lines.Count) top-level windows of pid $($d.Id); titled: $($titled -join ' // '))"
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
      # Up to 0.8.1: Back/Forward, a sidebar toggle and the page title
      # "Device Editor" in the toolbar. Later: none of them; the editor
      # keeps a visually hidden heading "Device Editor".
      $toolbar = Ui-All $main { (Ui-Name $_) -eq 'Toolbar' -and (Ui-Type $_) -ne 'Button' } | Select-Object -First 1
      $inToolbar = [bool]($toolbar -and (Ui-HasText $toolbar 'Device Editor'))
      $heading = [bool](Ui-All $main { (Ui-Name $_) -eq 'Device Editor' -and (Ui-Type $_) -notin 'Button', 'ToolTip', 'ListItem', 'MenuItem' } | Select-Object -First 1)
      $history = @('Back', 'Forward' | Where-Object { $names -contains $_ })
      $toggle = $names -contains 'Toggle Sidebar'
      $variant = if ($history -or $inToolbar -or $toggle) {
        "UI up to 0.8.1: $(if ($history) { $history -join '/' } else { 'no Back/Forward' }), $(if ($toggle) { 'sidebar toggle' } else { 'no sidebar toggle' }), $(if ($inToolbar) { "page title 'Device Editor' in the toolbar" } else { 'no page title in the toolbar' })"
      } else {
        "new UI: no Back/Forward, no sidebar toggle, no title in the toolbar ($(if ($toolbar) { "toolbar group 'Toolbar'" } else { 'toolbar group not exposed' })); $(if ($heading) { "page heading 'Device Editor'" } else { "no page heading 'Device Editor' exposed" })"
      }
      $rail = @($names | Where-Object { $_ -in 'Device Editor', 'Layouts', 'Dashboard', 'Action Library', 'Workflows' })
      "$($seen -join '; '); $variant; rail: $(if ($rail) { $rail -join ', ' } else { 'labels not exposed' }); all buttons: $($names -join ' | ')"
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
      $status = Ui-Until "'Installed $($script:PluginVersion)' in the install status" {
        $group = Ui-All $main { (Ui-Name $_) -eq 'Install status' } | Select-Object -First 1
        if ($group) { $t = Ui-Joined $group; if ($t.Contains("Installed $($script:PluginVersion)")) { $t } }
      } $script:UiLong
      "$boxes consent box(es) checked; install status '$status'; $consentShot; $(Ui-Shot 'installed')"
    }
    'Settings > Plugins lists it' = {
      $main = $state.main
      $visibleBefore, $titledBefore = Ui-WindowCounts
      $settingsButton = Ui-Find $main 'the Settings button' { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -like 'Settings*' }
      $pressedBefore = Ui-Pressed $settingsButton
      [void](Ui-Invoke $settingsButton)
      $how = 'Invoke'
      $where = Ui-SettingsAppeared $main 15
      if (-not $where) {
        # In case UIA's Invoke did not reach the button's click handler: a
        # real click on it, then the full wait.
        $how = "Invoke, then $(Ui-Click $main $settingsButton)"
        $where = Ui-SettingsAppeared $main
        if (-not $where) { throw "timed out after $($script:UiLong) s waiting for the settings: neither a window 'DeckPilot Settings' (up to 0.8.1) nor the settings view in the main window" }
      }
      if ($where -eq 'view') {
        # The settings as a view of the main window; the Marketplace
        # (Memory's details) waits under them and comes back as it was.
        $marker = Ui-SettingsView $main
        $windows = Ui-NoNewWindow $main $visibleBefore $titledBefore
        $pressed = Ui-Pressed (Ui-Gear $main)
        if ($pressed -eq 'not pressed') { throw 'the Settings button is not pressed while the settings are shown' }
        [void](Ui-Invoke (Ui-Button $main 'Plugins'))
        [void](Ui-Until "$($script:PluginName) v$($script:PluginVersion) in the installed list of Settings > Plugins" {
          $list = Ui-All $main { (Ui-Name $_) -eq 'Installed plugins' -and (Ui-Type $_) -ne 'Button' } | Select-Object -First 1
          $list -and (Ui-HasText $list $script:PluginName) -and (Ui-HasText $list "v$($script:PluginVersion)")
        })
        $shot = Ui-Shot 'settings-plugins'
        [void](Ui-Invoke (Ui-Button $main 'Marketplace'))
        [void](Ui-Until 'the settings view to close' { -not (Ui-SettingsView $main) })
        $status = Ui-Until "the Marketplace as it was before the settings: $($script:PluginName)'s install status" {
          $group = Ui-All $main { (Ui-Name $_) -eq 'Install status' } | Select-Object -First 1
          if ($group) { $t = Ui-Joined $group; if ($t.Contains("Installed $($script:PluginVersion)")) { $t } }
        }
        $after = Ui-Pressed (Ui-Gear $main)
        if ($after -eq 'pressed') { throw 'the Settings button stays pressed after leaving the settings' }
        return "new UI: the Settings button ($how; $pressedBefore before) showed the settings in the main window ($marker); $windows; Settings button $pressed; Extensions > Plugins lists $($script:PluginName) v$($script:PluginVersion); the toolbar's Marketplace left them: the Marketplace is back as it was (install status '$status'), Settings button $after; $shot; $(Ui-Shot 'settings-left')"
      }
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
      "UI up to 0.8.1: Settings button ($how) opened the separate top-level window 'DeckPilot Settings' (hwnd $($state.settingsHwnd), main window hwnd $($main.Current.NativeWindowHandle)); Extensions > Plugins lists $($script:PluginName) v$($script:PluginVersion); closed; $shot"
    }
    'enable with consent' = {
      $main = $state.main
      [void](Ui-Marketplace $main)
      [void](Ui-Tab $main 'Installed')
      # From 0.9.0 (plugin API 1.4) the install plan activates a plugin whose
      # install consent covered all its permissions: disable it here first so
      # enabling it again goes through the consent review as before.
      $enableName = "Enable $($script:PluginName)"
      $disableName = "Disable $($script:PluginName)"
      $atInstall = Ui-Find $main "switch '$enableName' or '$disableName'" { (Ui-Name $_) -eq $enableName -or (Ui-Name $_) -eq $disableName }
      $disabled = ''
      if ((Ui-Name $atInstall) -eq $disableName) {
        [void](Ui-Invoke $atInstall)
        $disabled = 'enabled at install, disabled with its switch; '
      }
      $switch = Ui-Find $main "switch '$enableName'" { (Ui-Name $_) -eq $enableName } $script:UiLong
      [void](Ui-Invoke $switch)
      $confirm = Ui-Button $main 'Enable'
      $consentShot = Ui-Shot 'enable-consent'
      $boxes = Ui-CheckAll $main
      [void](Ui-Invoke $confirm)
      $on = Ui-Find $main "switch 'Disable $($script:PluginName)'" { (Ui-Name $_) -eq "Disable $($script:PluginName)" } $script:UiLong
      "$disabled$boxes consent box(es); switch now '$(Ui-Name $on)'; $consentShot; $(Ui-Shot 'enabled')"
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
      # The alert dialog "Uninstall <name>?" is modal: the page behind it
      # can leave the accessibility tree while it is open (as it does on
      # macOS, run 44539186747), so its button is looked up inside the
      # dialog; else it is the Uninstall button that came after the list's.
      $dialogButton = Ui-Until "the confirmation dialog 'Uninstall $($script:PluginName)?'" {
        $box = Ui-All $main { (Ui-Name $_) -eq "Uninstall $($script:PluginName)?" -and (Ui-Type $_) -notin 'Text', 'Header' } | Select-Object -First 1
        $own = if ($box) { Ui-All $box { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Uninstall' } | Select-Object -First 1 }
        if ($own) { return $own }
        $all = @(Ui-All $main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Uninstall' })
        if ($all.Count -gt @($buttons).Count) { $all[-1] }
      }
      [void](Ui-Invoke $dialogButton)
      [void](Ui-Until "$($script:PluginName) to leave the installed list" {
        $list = Ui-All $main { (Ui-Name $_) -eq 'Installed plugins' } | Select-Object -First 1
        $list -and -not (Ui-HasText $list $script:PluginName)
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
      $visibleBefore, $titledBefore = Ui-WindowCounts
      [void](Ui-Invoke $create)
      $where = Ui-SettingsAppeared $main
      if (-not $where) { throw "timed out after $($script:UiLong) s waiting for Settings > Profiles: neither a window 'DeckPilot Settings' (up to 0.8.1) nor the settings view in the main window" }
      # Up to 0.8.1 the settings window; later the main window's settings view.
      $inWindow = $where -eq 'window'
      $settings = if ($inWindow) { Ui-SettingsWindow } else { $main }
      $windows = if ($inWindow) { '' } else { Ui-NoNewWindow $main $visibleBefore $titledBefore }
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
      if ($inWindow) {
        [void](Ui-Until "'$($script:ProfileName)' in All profiles" { -not (Ui-All $settings { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Save' } | Select-Object -First 1) -and (Ui-HasText $settings $script:ProfileName) })
        $shot = Ui-Shot 'settings-profiles'
        $w = Ui-Pattern $settings ([System.Windows.Automation.WindowPattern]::Pattern)
        if ($w) { $w.Close() }
        return "UI up to 0.8.1: Create a profile opened Settings > Profiles in the window 'DeckPilot Settings'; New profile '$($script:ProfileName)' saved and listed; $shot"
      }
      # The compact table (aria-label "Profiles"): each profile's name is a
      # button that makes it the profile being edited; the toolbar has no
      # profile selector in the settings, so a button of that name is the row.
      [void](Ui-Until "'$($script:ProfileName)' in the table of All profiles" {
        -not (Ui-All $main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq 'Save' } | Select-Object -First 1) -and
        (Ui-HasText $main 'All profiles') -and
        [bool](Ui-All $main { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -eq $script:ProfileName } | Select-Object -First 1)
      })
      $table = Ui-All $main { (Ui-Name $_) -eq 'Profiles' -and (Ui-Type $_) -in 'Table', 'DataGrid' } | Select-Object -First 1
      $shot = Ui-Shot 'settings-profiles'
      # Back where Create a profile was pressed: the rail's Device Editor.
      [void](Ui-Invoke (Ui-Button $main 'Device Editor'))
      [void](Ui-Until 'the settings view to close' { -not (Ui-SettingsView $main) })
      $selector = Ui-Find $main "the Device Editor's profile selector" { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -like 'Profile:*' }
      [void](Ui-Find $main "the Device Editor's device selector" { (Ui-Type $_) -eq 'Button' -and ((Ui-Name $_) -like 'Device:*' -or (Ui-Name $_) -eq 'Choose a device') })
      "new UI: Create a profile showed Settings > Profiles in the main window; $windows; New profile '$($script:ProfileName)' saved and listed in All profiles ($(if ($table) { "table 'Profiles'" } else { 'table not exposed as Table' }), row button '$($script:ProfileName)'); the rail's Device Editor left the settings: back in the Device Editor (device and profile selectors, '$(Ui-Name $selector)'); $shot; $(Ui-Shot 'settings-left-editor')"
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
      $modelNames = @($models | ForEach-Object { Ui-Name $_ })
      $modelName = Ui-Name $model
      $menuShot = Ui-Shot 'device-menu'
      [void](Ui-Invoke $model)
      # A new profile has no layout for the model: the editor offers "New
      # layout" (layout-designer.tsx); create one as a user would.
      $layout = 'a layout was there'
      $ready = Ui-Until "the device canvas or 'New layout'" {
        if (Ui-All $main { (Ui-Name $_) -like '* controls' -and (Ui-Type $_) -ne 'Button' } | Select-Object -First 1) { return 'canvas' }
        if (Ui-All $main { (Ui-Name $_) -like 'No layout for *' } | Select-Object -First 1) { return 'empty' }
      } $script:UiLong
      if ($ready -eq 'empty') {
        [void](Ui-Invoke (Ui-Button $main 'New layout'))
        [void](Ui-Find $main "the dialog 'New layout for ...'" { (Ui-Name $_) -like 'New layout for *' })
        $field = Ui-Find $main "the layout's Name field" { (Ui-Type $_) -eq 'Edit' -and (Ui-Name $_) -eq 'Name' }
        # Typed, so that the page's input handlers see it.
        Ui-Focus $main
        $field.SetFocus()
        [System.Windows.Forms.SendKeys]::SendWait('^a')
        [System.Windows.Forms.SendKeys]::SendWait($script:LayoutName)
        Start-Sleep -Milliseconds 500
        [void](Ui-Invoke (Ui-Button $main 'Create'))
        $layout = "created layout '$($script:LayoutName)' with New layout > Create"
      }
      $canvas = Ui-Find $main 'the device canvas' { (Ui-Name $_) -like '* controls' -and (Ui-Type $_) -ne 'Button' } $script:UiLong
      $library = Ui-Find $main 'the Action Library' { (Ui-Name $_) -eq 'Action Library' -and (Ui-Type $_) -ne 'Button' }
      # Up to 0.8.1 the library collapses and resizes; later it is fixed.
      $libraryControls = @(Ui-Texts $main | Where-Object { $_ -in 'Collapse the library', 'Resize the library', 'Show the library', 'Close the library' } | Sort-Object -Unique)
      $libraryKind = if ($libraryControls) { "with $($libraryControls -join ', ')" } else { 'fixed (no collapse or resize control)' }
      [void](Ui-Find $main 'the inspector' { (Ui-Name $_) -like 'Select a key, dial or touch zone*' })
      $key = Ui-Find $main "'Key 1'" { (Ui-Name $_) -eq 'Key 1' -and (Ui-Type $_) -ne 'Text' }
      $how = Ui-Click $main $key
      [void](Ui-Until "the inspector of Key 1" { -not (Ui-All $main { (Ui-Name $_) -like 'Select a key, dial or touch zone*' } | Select-Object -First 1) })
      $device = Ui-Find $main 'the device selector' { (Ui-Type $_) -eq 'Button' -and (Ui-Name $_) -like 'Device:*' }
      $state.editorRect = Ui-Rect $main
      "profile selector '$(Ui-Name $profile)'; device menu: $($modelNames -join ' | '); chose '$modelName' -> '$(Ui-Name $device)'; $layout; canvas '$(Ui-Name $canvas)'; library '$(Ui-Name $library)' ($(Ui-Type $library), $libraryKind); Key 1 selected ($how), the inspector shows it; $menuShot; $(Ui-Shot 'editor')"
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
  # What a step needs besides the main window: a failed step stops the
  # steps that need it. A failed Settings step alone does not stop the
  # steps that never use the settings window: the app is restarted (the
  # agent and the profile stay) and they run, so a defect of that window
  # does not hide the rest of the app.
  $needs = @{
    'profile created in Settings' = 'Settings > Plugins lists it'
    'Device Editor, Action Library and key inspector' = 'profile created in Settings'
    'reopening restores the window and the profile' = 'profile created in Settings'
  }
  $restartable = @('Settings > Plugins lists it')
  $failed = $null
  $skipped = @{}
  $restarted = $null
  foreach ($name in $steps.Keys) {
    $step = "${Label}: UIA $name"
    if ($failed) { Record $step 'not-covered' "not reached: '$failed' failed"; continue }
    $need = $needs[$name]
    if ($need -and $skipped.ContainsKey($need)) {
      $why = if ($skipped[$need] -eq 'failed') { 'failed' } else { 'was not covered' }
      $skipped[$name] = 'not covered'
      Record $step 'not-covered' "needs '$need', which $why"
      continue
    }
    try {
      $evidence = & $steps[$name]
      if ($restarted) { $evidence = "($restarted) $evidence"; $restarted = $null }
      if ($name -eq 'plugin toolbar entries') {
        Record $step 'not-covered' "no production plugin contributes toolbar entries (contributes.navigation) to show; with $($script:PluginName) enabled: $($state.navEvidence)"
      } else {
        Record $step 'pass' $evidence
      }
    } catch {
      $shot = Ui-Shot ("failure-" + ($name -replace '[^A-Za-z0-9]+', '-'))
      $dump = 'no window to dump'
      $d = Ui-Desktop
      if ($d) {
        $i = 0
        foreach ($w in (Ui-TopWindows $d.Id)) { $i++; $dump = Ui-Dump $w "$Label-ui-failure-tree-$i.tsv" }
      }
      Record $step 'fail' "$($_.Exception.Message); $shot; UIA tree: $dump; windows: $(Ui-WindowReport)"
      if ($name -notin $restartable) { $failed = $name; continue }
      $skipped[$name] = 'failed'
      try {
        $old = Ui-Desktop
        if ($old) { Stop-Process -Id $old.Id -Force; [void]$old.WaitForExit(15000) }
        $script:Desktop = Start-WithEnv $script:Exe $script:Envs
        $state.main = Ui-MainWindow
        $restarted = "deckpilot-desktop pid $(if ($old) { $old.Id } else { '?' }) ended after the failed step '$name' and started again as pid $($script:Desktop.Id)"
      } catch {
        $failed = "$name (and the restart after it: $($_.Exception.Message))"
      }
    }
  }
}
