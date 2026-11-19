// Drives the INSTALLED, notarized release app from outside through the
// macOS Accessibility API (AXUIElement), the layer VoiceOver uses:
//
//   swift build -c release --package-path tools/release-smoke/macos-ui
//   (or: swiftc -O -o macos-ui tools/release-smoke/macos-ui/Sources/macos-ui/main.swift)
//   macos-ui --probe --label <label> --out <dir>
//   macos-ui --label <label> --out <dir> --app <DeckPilot.app> --pid <deckpilot-desktop pid>
//
// Nothing in the app is changed or enabled for this: no WebDriver (none
// exists for WKWebView), no Web Inspector, no remote debugging. WebKit
// exposes the page's accessibility tree (roles, aria-labels) to AX
// clients, so elements are found by role and accessible name and pressed
// with AXPress; Radix menu triggers and the device canvas's keys, which
// react to pointer events, get a real click (CGEvent) at the centre of
// the element found by name, never at fixed coordinates. Window, Dock and
// activation-policy checks also use CGWindowList, NSRunningApplication and
// lsappinfo, which need no permission.
//
// --probe: what this runner grants (Accessibility, screen recording) and
// whether another process's AX tree can be read; changes nothing (no TCC
// database, no SIP, no setting).
//
// Each step is a line of <out>/results.tsv (tools/release-smoke/common.sh)
// named "<label>: AX <step>"; the first failed step stops the flow and the
// rest are recorded as not reached. Without Accessibility the AX steps are
// recorded as not covered and only the permission-free checks run.
import AppKit
import ApplicationServices
import CoreGraphics
import Foundation

setvbuf(stdout, nil, _IOLBF, 0)

// MARK: - arguments, results

var options: [String: String] = [:]
var flags: Set<String> = []
do {
  var i = 1
  let argv = CommandLine.arguments
  while i < argv.count {
    let a = argv[i]
    if a.hasPrefix("--") {
      if i + 1 < argv.count, !argv[i + 1].hasPrefix("--") {
        options[String(a.dropFirst(2))] = argv[i + 1]
        i += 2
        continue
      }
      flags.insert(String(a.dropFirst(2)))
    }
    i += 1
  }
}
let label = options["label"] ?? "app"
let out = options["out"] ?? FileManager.default.currentDirectoryPath
let platform = ProcessInfo.processInfo.environment["SMOKE_PLATFORM"] ?? "macos"
let pluginName = "Memory"
let pluginVersion = "0.1.0"
let profileName = "Release smoke"
let layoutName = "Release smoke layout"
let wait: TimeInterval = 30
let long: TimeInterval = 120
var shots = 0

func record(_ step: String, _ result: String, _ evidence: String) {
  let clean = evidence.replacingOccurrences(of: "[\t\r\n]+", with: " ", options: .regularExpression)
  let cut = String(clean.prefix(900))
  let line = "\(platform)\t\(label): AX \(step)\t\(result)\t\(cut)\n"
  let path = (out as NSString).appendingPathComponent("results.tsv")
  if let h = FileHandle(forWritingAtPath: path) {
    h.seekToEndOfFile()
    h.write(line.data(using: .utf8)!)
    h.closeFile()
  } else {
    FileManager.default.createFile(atPath: path, contents: line.data(using: .utf8))
  }
  print("\(result.uppercased())  AX \(step): \(cut)")
}

struct Failure: Error, CustomStringConvertible {
  let description: String
  init(_ d: String) { description = d }
}

@discardableResult
func sh(_ argv: [String]) -> (Int32, String) {
  let p = Process()
  p.executableURL = URL(fileURLWithPath: argv[0])
  p.arguments = Array(argv.dropFirst())
  let pipe = Pipe()
  p.standardOutput = pipe
  p.standardError = pipe
  do { try p.run() } catch { return (-1, "\(error)") }
  let data = pipe.fileHandleForReading.readDataToEndOfFile()
  p.waitUntilExit()
  return (p.terminationStatus, String(data: data, encoding: .utf8) ?? "")
}

/// Sleeps while letting AppKit update NSRunningApplication.
func pause(_ seconds: TimeInterval) {
  RunLoop.current.run(until: Date().addingTimeInterval(seconds))
}

func until<T>(_ what: String, _ timeout: TimeInterval = wait, _ probe: () throws -> T?) throws -> T {
  let deadline = Date().addingTimeInterval(timeout)
  var last: Error?
  while true {
    do {
      if let v = try probe() { return v }
    } catch { last = error }
    if Date() > deadline {
      throw Failure("timed out after \(Int(timeout)) s waiting for \(what)\(last.map { " (last error: \($0))" } ?? "")")
    }
    pause(0.5)
  }
}

func screenshot(_ name: String) -> String {
  shots += 1
  let file = String(format: "%@-ax-%02d-%@.png", label, shots, name)
  let (status, output) = sh(["/usr/sbin/screencapture", "-x", (out as NSString).appendingPathComponent(file)])
  if status == 0, FileManager.default.fileExists(atPath: (out as NSString).appendingPathComponent(file)) { return file }
  return "(screenshot \(name) failed: \(output.trimmingCharacters(in: .whitespacesAndNewlines)))"
}

// MARK: - accessibility

func attr(_ e: AXUIElement, _ name: String) -> AnyObject? {
  var v: CFTypeRef?
  return AXUIElementCopyAttributeValue(e, name as CFString, &v) == .success ? v : nil
}

func string(_ e: AXUIElement, _ name: String) -> String { (attr(e, name) as? String) ?? "" }
func role(_ e: AXUIElement) -> String { string(e, kAXRoleAttribute) }
func subrole(_ e: AXUIElement) -> String { string(e, kAXSubroleAttribute) }
func children(_ e: AXUIElement) -> [AXUIElement] { (attr(e, kAXChildrenAttribute) as? [AXUIElement]) ?? [] }
func enabled(_ e: AXUIElement) -> Bool { (attr(e, kAXEnabledAttribute) as? Bool) ?? true }

/// The accessible name: the title, else the description (aria-label),
/// else a text value.
func name(_ e: AXUIElement) -> String {
  let t = string(e, kAXTitleAttribute)
  if !t.isEmpty { return t }
  let d = string(e, kAXDescriptionAttribute)
  if !d.isEmpty { return d }
  return string(e, kAXValueAttribute)
}

func all(_ root: AXUIElement, limit: Int = 40000) -> [AXUIElement] {
  var queue = [root]
  var i = 0
  while i < queue.count, queue.count < limit {
    queue.append(contentsOf: children(queue[i]))
    i += 1
  }
  return queue
}

func first(_ root: AXUIElement, _ test: (AXUIElement) -> Bool) -> AXUIElement? {
  all(root).first(where: test)
}

func find(_ root: AXUIElement, _ what: String, _ timeout: TimeInterval = wait, _ test: @escaping (AXUIElement) -> Bool) throws -> AXUIElement {
  try until(what, timeout) { first(root, test) }
}

/// Something pressable: a button, a toggle button (aria-pressed: AXCheckBox
/// AXToggle), a menu trigger (aria-haspopup: AXPopUpButton).
func isButton(_ e: AXUIElement) -> Bool {
  let r = role(e)
  return r == kAXButtonRole as String || r == kAXPopUpButtonRole as String || r == kAXMenuButtonRole as String
    || (r == kAXCheckBoxRole as String && subrole(e) == "AXToggle")
}

func button(_ root: AXUIElement, _ title: String, _ timeout: TimeInterval = wait) throws -> AXUIElement {
  try find(root, "button '\(title)'", timeout) { isButton($0) && name($0) == title }
}

func press(_ e: AXUIElement) throws {
  let r = AXUIElementPerformAction(e, kAXPressAction as CFString)
  if r != .success { throw Failure("AXPress on '\(name(e))' (\(role(e))) failed: \(r.rawValue)") }
  pause(0.3)
}

func frame(_ e: AXUIElement) -> CGRect? {
  guard let p = attr(e, kAXPositionAttribute), let s = attr(e, kAXSizeAttribute) else { return nil }
  var point = CGPoint.zero
  var size = CGSize.zero
  AXValueGetValue(p as! AXValue, .cgPoint, &point)
  AXValueGetValue(s as! AXValue, .cgSize, &size)
  return CGRect(origin: point, size: size)
}

func describe(_ r: CGRect?) -> String {
  guard let r else { return "?" }
  return "\(Int(r.origin.x)),\(Int(r.origin.y)) \(Int(r.width))x\(Int(r.height))"
}

/// A real click at the centre of the element (found by name).
func click(_ app: NSRunningApplication?, _ e: AXUIElement) throws -> String {
  app?.activate()
  pause(0.4)
  guard let r = frame(e), r.width > 0 else { throw Failure("'\(name(e))' has no frame") }
  let point = CGPoint(x: r.midX, y: r.midY)
  let source = CGEventSource(stateID: .hidSystemState)
  for type in [CGEventType.mouseMoved, .leftMouseDown, .leftMouseUp] {
    CGEvent(mouseEventSource: source, mouseType: type, mouseCursorPosition: point, mouseButton: .left)?.post(tap: .cghidEventTap)
    pause(0.08)
  }
  pause(0.3)
  return "click at (\(Int(point.x)),\(Int(point.y)))"
}

func type(_ text: String) {
  let source = CGEventSource(stateID: .hidSystemState)
  for ch in text.utf16 {
    var c = ch
    for down in [true, false] {
      let ev = CGEvent(keyboardEventSource: source, virtualKey: 0, keyDown: down)
      ev?.keyboardSetUnicodeString(stringLength: 1, unicodeString: &c)
      ev?.post(tap: .cghidEventTap)
    }
    pause(0.03)
  }
}

func escape() {
  let source = CGEventSource(stateID: .hidSystemState)
  for down in [true, false] { CGEvent(keyboardEventSource: source, virtualKey: 53, keyDown: down)?.post(tap: .cghidEventTap) }
  pause(0.3)
}

func windows(_ app: AXUIElement) -> [AXUIElement] { (attr(app, kAXWindowsAttribute) as? [AXUIElement]) ?? [] }

func dump(_ root: AXUIElement, _ file: String) -> String {
  var lines: [String] = []
  for e in all(root) {
    lines.append("\(role(e))\t\(subrole(e))\t\(string(e, kAXTitleAttribute))\t\(string(e, kAXDescriptionAttribute))\t\(String(string(e, kAXValueAttribute).prefix(80)))\t\(describe(frame(e)))")
  }
  try? lines.joined(separator: "\n").write(toFile: (out as NSString).appendingPathComponent(file), atomically: true, encoding: .utf8)
  return "\(file) (\(lines.count) elements)"
}

// MARK: - windows, Dock, processes (no permission needed)

/// On-screen, normal-level windows of a process (CGWindowList).
func screenWindows(_ pid: pid_t) -> [CGRect] {
  let list = (CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]]) ?? []
  return list.compactMap { w in
    guard (w[kCGWindowOwnerPID as String] as? Int).map({ pid_t($0) }) == pid,
      (w[kCGWindowLayer as String] as? Int) == 0,
      let b = w[kCGWindowBounds as String] as? [String: Any],
      let rect = CGRect(dictionaryRepresentation: b as CFDictionary)
    else { return nil }
    return rect
  }
}

/// Windows of any layer a process owns on screen (a status item is one).
func statusWindows(_ pid: pid_t) -> [String] {
  let list = (CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]]) ?? []
  return list.compactMap { w in
    guard (w[kCGWindowOwnerPID as String] as? Int).map({ pid_t($0) }) == pid else { return nil }
    let layer = w[kCGWindowLayer as String] as? Int ?? -1
    let b = (w[kCGWindowBounds as String] as? [String: Any]).flatMap { CGRect(dictionaryRepresentation: $0 as CFDictionary) }
    return "layer \(layer) \(describe(b))"
  }
}

func policy(_ pid: pid_t) -> String {
  pause(0.2)
  switch NSRunningApplication(processIdentifier: pid)?.activationPolicy {
  case .regular: return "regular"
  case .accessory: return "accessory"
  case .prohibited: return "prohibited"
  default: return "unknown"
  }
}

/// lsappinfo's application type: Foreground (in the Dock), UIElement, BackgroundOnly.
func lsType(_ pid: pid_t) -> String {
  let (_, text) = sh(["/usr/bin/lsappinfo", "list"])
  for line in text.split(separator: "\n") where line.contains("pid = \(pid) ") {
    if let r = line.range(of: "type=\"") {
      return String(line[r.upperBound...].prefix { $0 != "\"" })
    }
  }
  return "?"
}

func dockItems() -> [String]? {
  guard let dock = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.dock").first else { return nil }
  let ax = AXUIElementCreateApplication(dock.processIdentifier)
  let items = all(ax, limit: 2000).filter { subrole($0) == "AXApplicationDockItem" }.map { name($0) }
  return items.isEmpty ? nil : items
}

func alive(_ pid: pid_t) -> Bool { kill(pid, 0) == 0 }

func agentPid() -> pid_t? {
  let (_, text) = sh(["/usr/bin/pgrep", "-x", "deckpilot-agent"])
  return text.split(separator: "\n").compactMap { pid_t($0) }.first
}

/// The production catalog the agent fetched into the isolated profile.
func catalogNames() -> [String] {
  guard let data = ProcessInfo.processInfo.environment["DECKPILOT_DATA_DIR"],
    let raw = FileManager.default.contents(atPath: (data as NSString).appendingPathComponent("plugin-registry/registry-index.json")),
    let json = try? JSONSerialization.jsonObject(with: raw) as? [String: Any]
  else { return [] }
  let signed = json["signed"] as? [String: Any]
  let plugins = (json["plugins"] ?? signed?["plugins"]) as? [[String: Any]] ?? []
  return Array(Set(plugins.compactMap { $0["name"] as? String })).sorted()
}

// MARK: - probe

if flags.contains("probe") {
  let trusted = AXIsProcessTrusted()
  var chain: [String] = []
  var pid = getpid()
  for _ in 0..<8 {
    let (_, line) = sh(["/bin/ps", "-o", "ppid=,comm=", "-p", "\(pid)"])
    let parts = line.trimmingCharacters(in: .whitespacesAndNewlines).split(separator: " ", maxSplits: 1)
    guard parts.count == 2, let parent = pid_t(parts[0]) else { break }
    chain.append("\(pid) \(parts[1])")
    if parent <= 1 { break }
    pid = parent
  }
  record("probe: AXIsProcessTrusted", trusted ? "pass" : "not-covered", "AXIsProcessTrusted() = \(trusted); process chain: \(chain.joined(separator: " <- "))")
  var finderError = "no Finder"
  if let finder = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.finder").first {
    var v: CFTypeRef?
    let r = AXUIElementCopyAttributeValue(AXUIElementCreateApplication(finder.processIdentifier), kAXChildrenAttribute as CFString, &v)
    finderError = "AXUIElementCopyAttributeValue(Finder, AXChildren) = \(r.rawValue) (\((v as? [AXUIElement])?.count ?? 0) children)"
  }
  let dock = dockItems()
  record("probe: read another app's AX tree", dock != nil ? "pass" : "not-covered", "\(finderError); Dock items: \(dock?.joined(separator: ", ") ?? "unreadable")")
  let capture = CGPreflightScreenCaptureAccess()
  let shot = screenshot("probe")
  record("probe: screen recording", capture ? "pass" : "not-covered", "CGPreflightScreenCaptureAccess() = \(capture); \(shot)")
  let (status, events) = sh(["/usr/bin/osascript", "-e", "tell application \"System Events\" to get name of every process whose background only is false"])
  record("probe: System Events (Apple Events + Accessibility)", status == 0 ? "pass" : "not-covered", "osascript exit \(status): \(events.trimmingCharacters(in: .whitespacesAndNewlines))")
  exit(0)
}

// MARK: - the flow

guard let pidText = options["pid"], let desktop = pid_t(pidText), let bundle = options["app"] else {
  print("usage: macos-ui --label <label> --out <dir> --app <DeckPilot.app> --pid <pid>")
  exit(2)
}
AXUIElementSetMessagingTimeout(AXUIElementCreateSystemWide(), 10)
let app = AXUIElementCreateApplication(desktop)
let running = NSRunningApplication(processIdentifier: desktop)
let trusted = AXIsProcessTrusted()
var state: [String: String] = [:]
var mainWindow: AXUIElement!
var mainFrame: CGRect?

func main(_ timeout: TimeInterval = long) throws -> AXUIElement {
  try until("the main window with its toolbar", timeout) {
    windows(app).first { w in first(w) { isButton($0) && name($0) == "Marketplace" } != nil }
  }
}

func settingsWindow(_ timeout: TimeInterval = long) throws -> AXUIElement {
  try until("the settings window (DeckPilot Settings)", timeout) {
    windows(app).first { string($0, kAXTitleAttribute) == "DeckPilot Settings" }
  }
}

/// The agent's menu bar item (an NSStatusItem of tray-icon).
func agentMenuBarItem() throws -> (pid_t, AXUIElement, Int) {
  guard let agent = agentPid() else { throw Failure("no deckpilot-agent process") }
  let ax = AXUIElementCreateApplication(agent)
  guard let extras = attr(ax, "AXExtrasMenuBar") else { throw Failure("the agent (pid \(agent)) has no AXExtrasMenuBar; its windows: \(statusWindows(agent).joined(separator: ", "))") }
  let items = children(extras as! AXUIElement)
  guard let item = items.first else { throw Failure("the agent's AXExtrasMenuBar has no item") }
  return (agent, item, items.count)
}

/// Opens the agent's menu with a real click on its menu bar item. tray-icon
/// shows the menu from the item's mouseDown handler (it attaches the menu
/// only for that click), so AXPress, which only performs the button's
/// action, opens nothing (run 44488294820).
func openAgentMenu(_ item: AXUIElement) throws -> (AXUIElement, String) {
  let how = try click(nil, item)
  let menu = try until("the agent's menu after a click on its menu bar item", 10) { children(item).first { role($0) == kAXMenuRole as String } }
  return (menu, how)
}

func isTab(_ e: AXUIElement, _ title: String) -> Bool {
  (role(e) == "AXRadioButton" || subrole(e) == "AXTabButton" || role(e) == "AXTab") && name(e).hasPrefix(title)
}

func marketplace(_ w: AXUIElement) throws {
  if let back = first(w, { isButton($0) && name($0) == "Back to plugins" }) { try press(back) }
  if first(w, { isTab($0, "Discover") }) == nil { try press(try button(w, "Marketplace")) }
  _ = try find(w, "the Marketplace's tabs") { isTab($0, "Discover") }
}

/// Radix tabs switch on mouse down or focus.
func tab(_ w: AXUIElement, _ title: String) throws {
  let t = try find(w, "tab '\(title)'") { isTab($0, title) }
  AXUIElementSetAttributeValue(t, kAXFocusedAttribute as CFString, kCFBooleanTrue)
  pause(0.5)
  if (attr(t, kAXValueAttribute) as? Int) != 1 { try press(t) }
  pause(0.5)
}

func checkAll(_ w: AXUIElement) throws -> Int {
  var n = 0
  for box in all(w).filter({ role($0) == kAXCheckBoxRole as String && subrole($0) != "AXSwitch" }) where (attr(box, kAXValueAttribute) as? Int) == 0 {
    try press(box)
    n += 1
  }
  return n
}

func card(_ w: AXUIElement) -> AXUIElement? {
  first(w) { isButton($0) && (name($0) == pluginName || name($0).hasPrefix("\(pluginName) ") || string($0, kAXDescriptionAttribute).hasPrefix(pluginName)) }
}

/// The text of an element and its descendants in tree order: WebKit
/// splits text into runs ("Installed " + "0.1.0").
func joined(_ root: AXUIElement) -> String {
  all(root).map(name).joined().replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
}

func hasText(_ root: AXUIElement, _ text: String) -> Bool { first(root) { name($0) == text } != nil || joined(root).contains(text) }

let axSteps: [(String, () throws -> String)] = [
  ("main window", {
    mainWindow = try main()
    mainFrame = frame(mainWindow)
    let web = first(mainWindow) { role($0) == "AXWebArea" }
    return "AX window '\(string(mainWindow, kAXTitleAttribute))' of deckpilot-desktop pid \(desktop) at \(describe(mainFrame)); web content exposed through AX (\(web != nil ? "AXWebArea" : "no AXWebArea")); \(screenshot("main"))"
  }),
  ("toolbar", {
    let names = all(mainWindow).filter(isButton).map(name).filter { !$0.isEmpty }
    let want: [(String, (String) -> Bool)] = [
      ("Marketplace", { $0 == "Marketplace" }),
      ("Settings", { $0.hasPrefix("Settings") }),
      ("device selector", { $0.hasPrefix("Device:") || $0 == "Choose a device" }),
      ("profile selector", { $0.hasPrefix("Profile:") || $0 == "Choose the profile to edit" }),
    ]
    var seen: [String] = []
    var missing: [String] = []
    for (what, test) in want {
      if let hit = names.first(where: test) { seen.append("\(what) '\(hit)'") } else { missing.append(what) }
    }
    if !missing.isEmpty { throw Failure("missing in the toolbar: \(missing.joined(separator: ", ")); buttons: \(names.joined(separator: " | "))") }
    let title = hasText(mainWindow, "Device Editor") ? "page title 'Device Editor'" : "no page title"
    let rail = names.filter { ["Device Editor", "Dashboard", "Action Library", "Workflows"].contains($0) }
    return "\(seen.joined(separator: "; ")); \(title); rail: \(rail.isEmpty ? "labels not exposed" : rail.joined(separator: ", ")); all buttons: \(names.joined(separator: " | "))"
  }),
  ("menu bar item of the agent", {
    let (agent, item, count) = try agentMenuBarItem()
    let itemName = [string(item, kAXTitleAttribute), string(item, kAXDescriptionAttribute), string(item, kAXHelpAttribute)].filter { !$0.isEmpty }.joined(separator: " / ")
    let (menu, how) = try openAgentMenu(item)
    let entries = children(menu).map { string($0, kAXTitleAttribute) }.filter { !$0.isEmpty }
    let shot = screenshot("menu-bar-menu")
    AXUIElementPerformAction(menu, kAXCancelAction as CFString)
    escape()
    for want in ["Quit DeckPilot"] where !entries.contains(want) { throw Failure("menu without '\(want)': \(entries.joined(separator: " | "))") }
    guard entries.contains(where: { $0.hasPrefix("Open DeckPilot") }) else { throw Failure("menu without 'Open DeckPilot': \(entries.joined(separator: " | "))") }
    return "deckpilot-agent pid \(agent): menu bar item '\(itemName)' (\(count) item(s)), opened by a \(how); its menu: \(entries.joined(separator: " | ")); \(shot)"
  }),
  ("Marketplace lists the production catalog", {
    try marketplace(mainWindow)
    try tab(mainWindow, "Discover")
    let c = try until("the '\(pluginName)' card", long) { card(mainWindow) }
    let catalog = catalogNames()
    let texts = all(mainWindow).map(name)
    let absent = catalog.filter { n in !texts.contains { $0 == n || $0.hasPrefix("\(n) ") } }
    if !catalog.isEmpty, !absent.isEmpty { throw Failure("catalog plugins not shown: \(absent.joined(separator: ", "))") }
    return "card '\(name(c).prefix(80))'; production catalog (registry-index.json in the profile): \(catalog.isEmpty ? "not readable" : catalog.joined(separator: ", ")); all shown; \(screenshot("marketplace"))"
  }),
  ("install \(pluginName) with consent", {
    guard let c = card(mainWindow) else { throw Failure("no card") }
    try press(c)
    try press(try button(mainWindow, "Install \(pluginVersion)"))
    let confirm = try button(mainWindow, "Install")
    let consentShot = screenshot("install-consent")
    let boxes = try checkAll(mainWindow)
    try press(confirm)
    _ = try until("the consent dialog to close") { first(mainWindow) { isButton($0) && name($0) == "Install" } == nil ? true : nil }
    let status = try until("'Installed \(pluginVersion)' in the install status", long) { () -> String? in
      guard let group = first(mainWindow, { name($0) == "Install status" }) else { return nil }
      let t = joined(group)
      return t.contains("Installed \(pluginVersion)") ? t : nil
    }
    return "\(boxes) consent box(es) checked; install status '\(status)'; \(consentShot); \(screenshot("installed"))"
  }),
  ("Settings opens its own window, Plugins lists it", {
    try press(try find(mainWindow, "the Settings button") { isButton($0) && name($0).hasPrefix("Settings") })
    let settings = try settingsWindow()
    let onScreen = screenWindows(desktop).count
    _ = try button(settings, "Back to DeckPilot")
    try press(try button(settings, "Plugins"))
    _ = try until("\(pluginName) v\(pluginVersion) in the plugin list") { hasText(settings, pluginName) && hasText(settings, "v\(pluginVersion)") ? true : nil }
    let shot = screenshot("settings-plugins")
    if let close = attr(settings, kAXCloseButtonAttribute) { try press(close as! AXUIElement) } else { try press(try button(settings, "Back to DeckPilot")) }
    _ = try until("the settings window to close") { windows(app).contains { string($0, kAXTitleAttribute) == "DeckPilot Settings" } ? nil : true }
    return "separate window 'DeckPilot Settings' (\(windows(app).count + 1) AX windows, \(onScreen) on screen while open); Extensions > Plugins lists \(pluginName) v\(pluginVersion); closed with its close button; \(shot)"
  }),
  ("enable with consent", {
    try marketplace(mainWindow)
    try tab(mainWindow, "Installed")
    try press(try find(mainWindow, "switch 'Enable \(pluginName)'") { name($0) == "Enable \(pluginName)" })
    let confirm = try button(mainWindow, "Enable")
    let consentShot = screenshot("enable-consent")
    let boxes = try checkAll(mainWindow)
    try press(confirm)
    let on = try find(mainWindow, "switch 'Disable \(pluginName)'", long) { name($0) == "Disable \(pluginName)" }
    return "\(boxes) consent box(es); switch now '\(name(on))'; \(consentShot); \(screenshot("enabled"))"
  }),
  ("plugin toolbar entries", {
    let nav = first(mainWindow) { name($0) == "Plugins" && !isButton($0) && role($0) != kAXListRole as String }
    state["nav"] = nav != nil ? "a 'Plugins' navigation is shown (\(role(nav!)))" : "no 'Plugins' navigation in the toolbar"
    return "skip"
  }),
  ("uninstall", {
    try marketplace(mainWindow)
    try tab(mainWindow, "Installed")
    let list = try find(mainWindow, "the installed list") { name($0) == "Installed plugins" }
    let before = all(mainWindow).filter { isButton($0) && name($0) == "Uninstall" }.count
    let item = all(list).first { (role($0) == "AXGroup" || role($0) == "AXListItem" || subrole($0) == "AXListItem") && hasText($0, pluginName) && first($0, { isButton($0) && name($0) == "Uninstall" }) != nil }
    guard let item, let own = first(item, { isButton($0) && name($0) == "Uninstall" }) else { throw Failure("no Uninstall button next to \(pluginName) in the installed list") }
    try press(own)
    // The alert dialog "Uninstall <name>?" is modal: the page behind it,
    // the list's own Uninstall button too, leaves the AX tree while it is
    // open (run 44539186747), so its button is looked up inside the dialog.
    let dialog = try until("the confirmation dialog 'Uninstall \(pluginName)?'") { () -> AXUIElement? in
      if let box = first(mainWindow, { subrole($0).hasSuffix("Dialog") && name($0) == "Uninstall \(pluginName)?" }),
        let confirm = first(box, { isButton($0) && name($0) == "Uninstall" })
      {
        return confirm
      }
      let buttons = all(mainWindow).filter { isButton($0) && name($0) == "Uninstall" }
      return buttons.count > before ? buttons.last : nil
    }
    let confirmShot = screenshot("uninstall-confirm")
    try press(dialog)
    _ = try until("\(pluginName) to leave the installed list", long) { () -> Bool? in
      guard let l = first(mainWindow, { name($0) == "Installed plugins" }) else { return nil }
      return hasText(l, pluginName) ? nil : true
    }
    return "\(pluginName) gone from Marketplace > Installed; \(confirmShot); \(screenshot("uninstalled"))"
  }),
  ("profile created in Settings", {
    var create = first(mainWindow) { isButton($0) && name($0) == "Create a profile" }
    if create == nil {
      if let editor = first(mainWindow, { isButton($0) && name($0) == "Device Editor" }) { try press(editor) }
      create = try button(mainWindow, "Create a profile")
    }
    try press(create!)
    let settings = try settingsWindow()
    try press(try button(settings, "New profile"))
    // WebKit names a labelled input by its label (AXTitle, or the
    // AXTitleUIElement's text); the dialog focuses the field on open.
    let field = try until("the profile's Name field") { () -> AXUIElement? in
      let fields = all(settings).filter { role($0) == kAXTextFieldRole as String }
      if let f = fields.first(where: { name($0) == "Name" }) { return f }
      if let f = fields.first(where: { e in (attr(e, "AXTitleUIElement")).map { name($0 as! AXUIElement) } == "Name" }) { return f }
      return fields.first { (attr($0, kAXFocusedAttribute) as? Bool) == true }
    }
    AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue)
    AXUIElementSetAttributeValue(field, kAXValueAttribute as CFString, profileName as CFString)
    pause(0.5)
    let save = try button(settings, "Save")
    var how = "AXValue"
    if !enabled(save) {
      NSRunningApplication(processIdentifier: desktop)?.activate()
      AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue)
      type(profileName)
      how = "typed"
      pause(0.5)
    }
    try press(save)
    _ = try until("'\(profileName)' in All profiles") { first(settings, { isButton($0) && name($0) == "Save" }) == nil && hasText(settings, profileName) ? true : nil }
    let shot = screenshot("settings-profiles")
    if let close = attr(settings, kAXCloseButtonAttribute) { try press(close as! AXUIElement) }
    return "Create a profile opened Settings > Profiles; New profile '\(profileName)' (\(how)) saved and listed; \(shot)"
  }),
  ("Device Editor, Action Library and key inspector", {
    var profile = try find(mainWindow, "the profile selector") { isButton($0) && (name($0).hasPrefix("Profile:") || name($0) == "Choose the profile to edit") }
    if !name(profile).hasPrefix("Profile: \(profileName)") {
      _ = try click(running, profile)
      try press(try find(mainWindow, "menu item '\(profileName)'") { role($0) == kAXMenuItemRole as String && name($0).contains(profileName) })
      profile = try find(mainWindow, "the profile selector on '\(profileName)'") { isButton($0) && name($0).hasPrefix("Profile: \(profileName)") }
    }
    let selector = try find(mainWindow, "the device selector", long) { isButton($0) && (name($0).hasPrefix("Device:") || name($0) == "Choose a device") && enabled($0) }
    _ = try click(running, selector)
    // The page's menu (in the web area), not the app's menu bar, whose
    // Apple menu items are AXMenuItems of the app too (run 44541282680
    // pressed "About This Mac").
    let models = try until("the device menu") { () -> [AXUIElement]? in
      let items = all(mainWindow).filter { role($0) == kAXMenuItemRole as String && enabled($0) && !name($0).isEmpty }
      return items.isEmpty ? nil : items
    }
    let model = models[0]
    let modelName = name(model)
    let modelNames = models.map(name)
    let menuShot = screenshot("device-menu")
    try press(model)
    // A new profile has no layout for the model: the editor offers "New
    // layout" (layout-designer.tsx); create one as a user would.
    var layout = "a layout was there"
    let isCanvas: (AXUIElement) -> Bool = { name($0).hasSuffix(" controls") && !isButton($0) }
    let ready = try until("the device canvas or 'New layout'", long) { () -> String? in
      if first(mainWindow, isCanvas) != nil { return "canvas" }
      return first(mainWindow, { name($0).hasPrefix("No layout for ") || string($0, kAXValueAttribute).hasPrefix("No layout for ") }) != nil ? "empty" : nil
    }
    if ready == "empty" {
      try press(try button(mainWindow, "New layout"))
      _ = try find(mainWindow, "the dialog 'New layout for …'") { name($0).hasPrefix("New layout for ") }
      let field = try until("the layout's Name field") { () -> AXUIElement? in
        let fields = all(mainWindow).filter { role($0) == kAXTextFieldRole as String }
        return fields.first { (attr($0, kAXFocusedAttribute) as? Bool) == true } ?? fields.first { name($0) == "Name" }
      }
      NSRunningApplication(processIdentifier: desktop)?.activate()
      AXUIElementSetAttributeValue(field, kAXFocusedAttribute as CFString, kCFBooleanTrue)
      type(layoutName)
      pause(0.5)
      try press(try button(mainWindow, "Create"))
      layout = "created layout '\(layoutName)' with New layout › Create"
    }
    let canvas = try find(mainWindow, "the device canvas", long, isCanvas)
    let library = try find(mainWindow, "the Action Library") { name($0) == "Action Library" && !isButton($0) }
    _ = try find(mainWindow, "the inspector") { name($0).hasPrefix("Select a key, dial or touch zone") }
    let key = try find(mainWindow, "'Key 1'") { name($0) == "Key 1" && role($0) != kAXStaticTextRole as String }
    let how = try click(running, key)
    _ = try until("the inspector of Key 1") { first(mainWindow, { name($0).hasPrefix("Select a key, dial or touch zone") }) == nil ? true : nil }
    let device = try find(mainWindow, "the device selector") { isButton($0) && name($0).hasPrefix("Device:") }
    mainFrame = frame(mainWindow)
    return "profile selector '\(name(profile))'; device menu: \(modelNames.joined(separator: " | ")); chose '\(modelName)' -> '\(name(device))'; \(layout); canvas '\(name(canvas))'; library '\(name(library))' (\(role(library)) \(subrole(library))); Key 1 selected (\(how)), the inspector shows it; \(menuShot); \(screenshot("editor"))"
  }),
  ("closing the main window leaves the Dock, the app and agent keep running", {
    let before = "policy \(policy(desktop)), lsappinfo \(lsType(desktop)), Dock: \(dockItems()?.contains("DeckPilot") == true ? "DeckPilot" : "no DeckPilot")"
    guard let close = attr(mainWindow, kAXCloseButtonAttribute) else { throw Failure("the main window has no close button") }
    try press(close as! AXUIElement)
    _ = try until("the main window to leave the screen") { screenWindows(desktop).isEmpty ? true : nil }
    _ = try until("the accessory activation policy") { policy(desktop) == "accessory" ? true : nil }
    pause(2)
    let dock = dockItems()
    if dock?.contains("DeckPilot") == true { throw Failure("DeckPilot is still in the Dock: \(dock!.joined(separator: ", "))") }
    guard alive(desktop) else { throw Failure("deckpilot-desktop ended") }
    guard let agent = agentPid() else { throw Failure("the agent ended") }
    return "before: \(before); after the close button: no on-screen window, policy \(policy(desktop)), lsappinfo \(lsType(desktop)), Dock: \(dock?.joined(separator: ", ") ?? "unreadable") (no DeckPilot); deckpilot-desktop pid \(desktop) and deckpilot-agent pid \(agent) running; \(screenshot("closed"))"
  }),
  ("reopening from the menu bar restores the window", {
    let (_, item, _) = try agentMenuBarItem()
    let (menu, _) = try openAgentMenu(item)
    let open = try until("'Open DeckPilot'", 10) { children(menu).first { string($0, kAXTitleAttribute).hasPrefix("Open DeckPilot") } }
    try press(open)
    let w = try main()
    mainWindow = w
    _ = try until("the regular activation policy") { policy(desktop) == "regular" ? true : nil }
    pause(1)
    let after = frame(w)
    let same = after == mainFrame ? "same frame as before closing" : "before closing \(describe(mainFrame))"
    let dock = dockItems()
    let profile = first(w) { isButton($0) && name($0).hasPrefix("Profile:") }.map(name) ?? "?"
    if dock?.contains("DeckPilot") != true { throw Failure("DeckPilot is not back in the Dock: \(dock?.joined(separator: ", ") ?? "unreadable")") }
    return "menu bar > Open DeckPilot: window at \(describe(after)) (\(same)), \(screenWindows(desktop).count) on screen; policy \(policy(desktop)), lsappinfo \(lsType(desktop)), DeckPilot back in the Dock; profile selector '\(profile)'; \(screenshot("reopened"))"
  }),
  ("reopening from Finder (open) after closing again", {
    guard let close = attr(mainWindow, kAXCloseButtonAttribute) else { throw Failure("no close button") }
    try press(close as! AXUIElement)
    _ = try until("the accessory activation policy") { policy(desktop) == "accessory" ? true : nil }
    // The running instance gets a reopen event; nothing new is launched.
    let (status, output) = sh(["/usr/bin/open", bundle])
    if status != 0 { throw Failure("open \(bundle): \(output)") }
    _ = try main()
    _ = try until("the regular activation policy") { policy(desktop) == "regular" ? true : nil }
    let instances = NSRunningApplication.runningApplications(withBundleIdentifier: "io.github.knittingcode.deckpilot").map { "\($0.processIdentifier)" }
    if instances != ["\(desktop)"] { throw Failure("instances after open: \(instances.joined(separator: ", "))") }
    return "open \((bundle as NSString).lastPathComponent): the same process (pid \(desktop)) showed its window again (\(describe(frame(try main())))), policy \(policy(desktop)); \(screenshot("reopened-open"))"
  }),
]

// Without Accessibility: what the permission-free APIs can show.
func withoutAccessibility() {
  let reason = "this runner does not grant Accessibility to the job (AXIsProcessTrusted() = false); the TCC database and SIP are not modified"
  let deadline = Date().addingTimeInterval(long)
  while screenWindows(desktop).isEmpty, Date() < deadline { pause(1) }
  let wins = screenWindows(desktop)
  record("main window (CGWindowList)", wins.isEmpty ? "fail" : "pass", "on-screen windows of pid \(desktop): \(wins.map(describe).joined(separator: ", ")); policy \(policy(desktop)); lsappinfo \(lsType(desktop)); \(screenshot("main"))")
  if let agent = agentPid() {
    record("menu bar item (CGWindowList)", "pass", "deckpilot-agent pid \(agent) windows: \(statusWindows(agent).joined(separator: ", "))")
  }
  for (step, _) in axSteps { record(step, "not-covered", reason) }
}

if !trusted {
  withoutAccessibility()
  exit(0)
}
record("accessibility", "pass", "AXIsProcessTrusted() = true for this job's process")
var failed: String?
for (step, run) in axSteps {
  if let failed {
    record(step, "not-covered", "not reached: '\(failed)' failed")
    continue
  }
  do {
    let evidence = try run()
    if step == "plugin toolbar entries" {
      record(step, "not-covered", "no production plugin contributes toolbar entries (contributes.navigation) to show; with \(pluginName) enabled: \(state["nav"] ?? "?")")
    } else {
      record(step, "pass", evidence)
    }
  } catch {
    failed = step
    let shot = screenshot("failure-" + step.replacingOccurrences(of: "[^A-Za-z0-9]+", with: "-", options: .regularExpression))
    var dumps: [String] = []
    for (i, w) in windows(app).enumerated() { dumps.append(dump(w, "\(label)-ax-failure-tree-\(i + 1).tsv")) }
    if let agent = agentPid(), let extras = attr(AXUIElementCreateApplication(agent), "AXExtrasMenuBar") {
      dumps.append(dump(extras as! AXUIElement, "\(label)-ax-failure-agent-menu-bar.tsv"))
      escape()
    }
    record(step, "fail", "\(error); \(shot); AX tree: \(dumps.isEmpty ? "no window" : dumps.joined(separator: ", "))")
  }
}
