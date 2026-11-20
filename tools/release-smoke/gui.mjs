// Drives the INSTALLED release app through tauri-driver (W3C WebDriver over
// HTTP, Node's built-in fetch, no dependencies) against the production
// plugin Marketplace:
//
//   node tools/release-smoke/gui.mjs --port <tauri-driver port> --app <executable> --label <label> --out <dir>
//
// The application shell of 0.8.0 and later: main window (toolbar) ›
// Marketplace › Discover › Memory (deckpilot.memory) › Install with consent
// › Settings › Extensions › Plugins lists it › back in the main window:
// Installed › Enable with consent › Uninstall. Up to 0.8.1 the settings are
// a window of their own ("Back to DeckPilot" returns); later releases show
// them in the main window (back with the toolbar). Both are accepted, and
// the step says which one the release has.
// The selectors are those of apps/desktop/src-tauri/tests/marketplace_gui.rs.
// Each step is recorded in <out>/results.tsv (see common.sh); a step that
// fails stops the flow, the rest are recorded as not reached. A session
// that cannot be created is recorded as not covered, with the driver's error.
import { execFileSync } from "node:child_process"
import { appendFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, arg, i, all) => {
    if (arg.startsWith("--")) pairs.push([arg.slice(2), all[i + 1]])
    return pairs
  }, []),
)
const { port, app, label, out } = args
const platform = process.env.SMOKE_PLATFORM ?? "unknown"
const ELEMENT = "element-6066-11e4-a52e-4f735466cecf"
const PLUGIN = "deckpilot.memory"
const NAME = "Memory"
const VERSION = "0.1.0"
const WAIT = 30_000
const LONG = 120_000
let shots = 0

function record(step, result, evidence) {
  const clean = String(evidence ?? "")
    .replace(/[\t\r\n]+/g, " ")
    .slice(0, 900)
  appendFileSync(
    join(out, "results.tsv"),
    `${platform}\t${label}: GUI ${step}\t${result}\t${clean}\n`,
  )
  console.log(`${result.toUpperCase()}  GUI ${step}: ${clean}`)
}

async function call(method, path, body, timeout = 180_000) {
  const response = await fetch(`http://127.0.0.1:${port}${path}`, {
    method,
    headers: body ? { "content-type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(timeout),
  })
  const text = await response.text()
  let answer
  try {
    answer = JSON.parse(text)
  } catch {
    throw new Error(`invalid answer (${response.status}): ${text.slice(0, 300)}`)
  }
  const value = answer.value
  if (value && typeof value === "object" && value.error) {
    throw new Error(`${value.error}: ${value.message}`)
  }
  return value
}

let session
const s = (path) => `/session/${session}/${path}`
const execute = (script, a = []) => call("POST", s("execute/sync"), { script, args: a })
const executeAsync = (script, a = []) => call("POST", s("execute/async"), { script, args: a })
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const lit = (t) => (t.includes("'") ? `"${t}"` : `'${t}'`)
const button = (t) => `//button[normalize-space(.)=${lit(t)}]`
const tab = (t) => `//button[@role='tab' and starts-with(normalize-space(.),${lit(t)})]`
const DIALOG = "//*[@role='dialog' or @role='alertdialog']"
/** The toolbar's Settings button (its label mentions an available update). */
const SETTINGS = "//header[@aria-label='Toolbar']//button[starts-with(@aria-label,'Settings')]"
/** The toolbar's Marketplace button. */
const MARKETPLACE = "//header[@aria-label='Toolbar']//button[@aria-label='Marketplace']"
/** The identifier of the settings window (apps/desktop/src-tauri/src/windows.rs). */
const SETTINGS_WINDOW = "app.settings"
/** The settings as a view of the main window (releases after 0.8.1): their sidebar. */
const SETTINGS_VIEW = "//*[@data-slot='sidebar' and @aria-label='Settings']"

async function findAll(xpath) {
  const found = await call("POST", s("elements"), { using: "xpath", value: xpath })
  return (found ?? []).map((e) => e[ELEMENT])
}

async function displayed(id) {
  return execute(
    "const e = arguments[0]; const r = e.getBoundingClientRect(); const st = getComputedStyle(e); " +
      "return r.width > 0 && r.height > 0 && st.visibility !== 'hidden' && st.display !== 'none';",
    [{ [ELEMENT]: id }],
  )
}

async function wait(xpath, timeout = WAIT) {
  const deadline = Date.now() + timeout
  let last
  for (;;) {
    try {
      for (const id of await findAll(xpath)) if (await displayed(id)) return id
    } catch (e) {
      last = e
    }
    if (Date.now() > deadline) {
      throw new Error(
        `no displayed element for ${xpath} within ${timeout / 1000} s${last ? ` (last error: ${last.message})` : ""}`,
      )
    }
    await sleep(250)
  }
}

async function visible(xpath) {
  for (const id of await findAll(xpath).catch(() => []))
    if (await displayed(id).catch(() => false)) return true
  return false
}

async function click(xpath, timeout = WAIT) {
  const deadline = Date.now() + timeout
  for (;;) {
    const id = await wait(xpath, Math.max(1000, deadline - Date.now()))
    await execute("arguments[0].scrollIntoView({block: 'center', inline: 'center'});", [
      { [ELEMENT]: id },
    ]).catch(() => {})
    try {
      await call("POST", s(`element/${id}/click`), {})
      return
    } catch (e) {
      if (Date.now() > deadline) throw e
      await sleep(300)
    }
  }
}

async function text(xpath) {
  const id = await wait(xpath)
  return call("GET", s(`element/${id}/text`))
}

async function invoke(command, a = {}) {
  const answer = await executeAsync(
    "const done = arguments[arguments.length - 1]; window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1]).then(" +
      "(v) => done({ok: v === undefined ? null : v}), (e) => done({err: e === undefined ? null : e}))",
    [command, a],
  )
  if (answer && "err" in answer) throw new Error(`${command}: ${JSON.stringify(answer.err)}`)
  return answer?.ok
}

async function until(what, f, timeout = LONG) {
  const deadline = Date.now() + timeout
  let last
  for (;;) {
    try {
      const v = await f()
      if (v) return v
    } catch (e) {
      last = e
    }
    if (Date.now() > deadline)
      throw new Error(
        `timed out waiting for ${what}${last ? ` (last error: ${last.message})` : ""}`,
      )
    await sleep(500)
  }
}

async function screenshot(name) {
  shots += 1
  const file = `${label}-gui-${String(shots).padStart(2, "0")}-${name}.png`
  try {
    const data = await call("GET", s("screenshot"), undefined, 30_000)
    writeFileSync(join(out, file), Buffer.from(data, "base64"))
    return file
  } catch (e) {
    console.log(`WebDriver screenshot ${name} failed: ${e.message}`)
    if (process.platform === "linux") {
      // The whole (virtual) screen instead.
      try {
        execFileSync("import", ["-window", "root", join(out, file)], { timeout: 30_000 })
        return `${file} (X screen; the WebDriver screenshot failed: ${e.message})`
      } catch {}
    }
    return `(screenshot failed: ${e.message})`
  }
}

async function consentAll() {
  const unchecked = `${DIALOG}//button[@role='checkbox' and @aria-checked='false']`
  let n = 0
  while ((await findAll(unchecked)).length > 0) {
    await click(unchecked)
    n += 1
    if (n > 10) throw new Error("the consent boxes do not stay checked")
  }
  return n
}

async function dialogClosed() {
  await until("the dialog to close", async () => !(await visible(DIALOG)), WAIT)
}

const handles = async () => (await call("GET", s("window/handles"))) ?? []
const switchTo = (handle) => call("POST", s("window"), { handle })
const windowId = () => execute("return document.documentElement.dataset.window || ''")

/** The main window (its toolbar), switched to; returns its handle. */
async function mainWindow(timeout = WAIT) {
  return until(
    "the main window's toolbar",
    async () => {
      for (const handle of await handles()) {
        await switchTo(handle)
        if ((await windowId()) === "" && (await visible(MARKETPLACE))) return handle
      }
      return null
    },
    timeout,
  )
}

/** The Marketplace (toolbar › Marketplace) at its tabs, unless open already. */
async function marketplace() {
  // A plugin's details replace the tabs until "Back to plugins".
  if (await visible(button("Back to plugins"))) await click(button("Back to plugins"))
  if (!(await visible(tab("Discover")))) {
    await click(MARKETPLACE)
    await wait(tab("Discover"))
  }
}

async function installedTab() {
  await marketplace()
  await click(tab("Installed"))
  await wait("//ul[@aria-label='Installed plugins']")
}

const card = `//ul[@aria-label='Plugins']//button[.//span[normalize-space(.)=${lit(NAME)}]]`
const installedItem = `//ul[@aria-label='Installed plugins']//li[.//span[normalize-space(.)=${lit(NAME)}]]`
let mainHandle

const steps = [
  [
    "main window",
    async () => {
      mainHandle = await mainWindow(LONG)
      // Up to 0.8.1 the toolbar shows the page title "Device Editor"; later
      // the toolbar has no page title and the rail marks the current page.
      const titled = "//header[@aria-label='Toolbar']//h1[normalize-space(.)='Device Editor']"
      await wait(
        `${titled} | //button[@aria-current='page'][.//span[normalize-space(.)='Device Editor']]`,
      )
      const editor = (await visible(titled))
        ? "UI up to 0.8.1: page title 'Device Editor' in the toolbar"
        : "new UI: no title in the toolbar; the rail marks Device Editor as the current page"
      await wait(SETTINGS)
      const status = await invoke("agent_status", {})
      return `main window shown (toolbar with Marketplace and Settings; ${editor}); agent_status: ${JSON.stringify(status).slice(0, 300)}; ${await screenshot("main")}`
    },
  ],
  [
    "toolbar > Marketplace",
    async () => {
      await marketplace()
      await wait("//h1[normalize-space(.)='Marketplace']")
      return `Marketplace shown in the main window (Discover tab); ${await screenshot("marketplace")}`
    },
  ],
  [
    "production catalog lists Memory",
    async () => {
      await click(tab("Discover"))
      try {
        await wait(card, 15_000)
      } catch {
        await click(button("Refresh"))
        await wait(card, LONG)
      }
      const available = await invoke("list_available_plugins", { refresh: false })
      const registry = available?.registry ?? {}
      const ids = (available?.plugins ?? []).map((p) => p.id)
      if (!ids.includes(PLUGIN))
        throw new Error(`${PLUGIN} not in list_available_plugins: ${ids.join(", ")}`)
      return `card '${NAME}' shown; registry ${registry.source} seq ${registry.seq} freshness ${registry.freshness}; plugins: ${ids.join(", ")}; ${await screenshot("catalog")}`
    },
  ],
  [
    `install ${PLUGIN} ${VERSION} with consent`,
    async () => {
      await click(card)
      const title = await text("//h3[@id='plugin-detail-name']")
      if (title.trim() !== NAME) throw new Error(`details show '${title}'`)
      await click(button(`Install ${VERSION}`))
      await wait(DIALOG)
      const consentShot = await screenshot("install-consent")
      const boxes = await consentAll()
      await click(`${DIALOG}${button("Install")}`)
      await dialogClosed()
      const status = await until(`Installed ${VERSION}`, async () => {
        const t = await text("//section[@aria-label='Install status']")
        return t.includes(`Installed ${VERSION}`) ? t : null
      })
      const plugin = await invoke("get_plugin", { plugin: PLUGIN })
      return `${boxes} consent box(es) checked; status '${status.trim()}'; get_plugin: version ${plugin?.summary?.version}, state ${plugin?.summary?.state}; ${consentShot}; ${await screenshot("installed")}`
    },
  ],
  [
    "Settings > Extensions > Plugins lists Memory",
    async () => {
      await switchTo(mainHandle)
      const before = new Set(await handles())
      await click(SETTINGS)
      // Either the main window shows the settings, or (up to 0.8.1) a
      // settings window appears.
      const where = await until(
        "the settings",
        async () => {
          await switchTo(mainHandle)
          if (await visible(SETTINGS_VIEW)) return { handle: mainHandle }
          for (const handle of await handles()) {
            if (handle === mainHandle) continue
            await switchTo(handle)
            if ((await windowId()) === SETTINGS_WINDOW) return { handle }
          }
          await switchTo(mainHandle)
          return null
        },
        LONG,
      )
      const inMain = where.handle === mainHandle
      let pressed = ""
      if (inMain) {
        await sleep(2000)
        const windows = (await handles()).length
        if (windows !== before.size)
          throw new Error(
            `the settings opened in the main window, but ${windows - before.size} more window(s) appeared`,
          )
        await switchTo(mainHandle)
        if (await visible(button("Back to DeckPilot")))
          throw new Error("the settings view has a 'Back to DeckPilot' button")
        pressed = await execute("return arguments[0].getAttribute('aria-pressed')", [
          { [ELEMENT]: await wait(SETTINGS) },
        ])
        if (pressed !== "true")
          throw new Error(
            `the Settings button is not pressed in the settings (aria-pressed ${pressed})`,
          )
      } else {
        await wait(button("Back to DeckPilot"))
      }
      // In the main window the Plugins entry of the settings' sidebar (the
      // workspace page under the settings is hidden).
      await click(inMain ? `${SETTINGS_VIEW}${button("Plugins")}` : button("Plugins"))
      const row = await wait(`${installedItem}//span[normalize-space(.)='v${VERSION}']`)
      await execute("arguments[0].scrollIntoView({block: 'center'});", [{ [ELEMENT]: row }])
      const shot = await screenshot("settings-plugins")
      let back = "back to the main window"
      if (inMain) {
        // The toolbar's Marketplace leaves the settings; the Marketplace
        // (Memory's details) waited under them and comes back as it was.
        await click(MARKETPLACE)
        await until("the settings view to close", async () => !(await visible(SETTINGS_VIEW)), WAIT)
        const status = await until(
          `the Marketplace as it was: Installed ${VERSION}`,
          async () => {
            const t = await text("//section[@aria-label='Install status']")
            return t.includes(`Installed ${VERSION}`) ? t : null
          },
          WAIT,
        )
        const after = await execute("return arguments[0].getAttribute('aria-pressed')", [
          { [ELEMENT]: await wait(SETTINGS) },
        ])
        if (after === "true")
          throw new Error("the Settings button stays pressed after leaving the settings")
        back = `the toolbar's Marketplace left them: the Marketplace is back as it was (install status '${status.trim()}'), Settings aria-pressed ${after}`
      } else {
        await click(button("Back to DeckPilot"))
        await switchTo(mainHandle)
      }
      await wait(MARKETPLACE)
      const how = inMain
        ? `new UI: settings shown in the main window (${SETTINGS_VIEW}; no other window, no 'Back to DeckPilot', Settings aria-pressed ${pressed})`
        : `UI up to 0.8.1: settings window ${before.has(where.handle) ? "(already open)" : "opened"} as its own window (${SETTINGS_WINDOW})`
      return `${how}; Extensions > Plugins lists ${NAME} v${VERSION}; ${back}; ${shot}`
    },
  ],
  [
    "enable with consent",
    async () => {
      await installedTab()
      // From 0.9.0 (plugin API 1.4) the install plan activates a plugin whose
      // install consent covered all its permissions: disable it here first so
      // enabling it again goes through the consent review as before.
      const enableSwitch = `//button[@role='switch' and @aria-label=${lit(`Enable ${NAME}`)}]`
      const disableSwitch = `//button[@role='switch' and @aria-label=${lit(`Disable ${NAME}`)}]`
      const atInstall = await until("the plugin's enable switch", async () =>
        (await visible(enableSwitch))
          ? "disabled"
          : (await visible(disableSwitch))
            ? "enabled"
            : null,
      )
      let disabled = ""
      if (atInstall === "enabled") {
        await click(disableSwitch)
        await wait(enableSwitch, LONG)
        const off = await until("state disabled", async () => {
          const p = await invoke("get_plugin", { plugin: PLUGIN })
          return p?.summary?.state === "disabled" ? p : null
        })
        disabled = `enabled at install, disabled with its switch (state ${off.summary.state}); `
      }
      await click(enableSwitch)
      await wait(DIALOG)
      const consentShot = await screenshot("enable-consent")
      const boxes = await consentAll()
      await click(`${DIALOG}${button("Enable")}`)
      await dialogClosed()
      await wait(disableSwitch, LONG)
      const plugin = await until("state enabled", async () => {
        const p = await invoke("get_plugin", { plugin: PLUGIN })
        return p?.summary?.state === "enabled" ? p : null
      })
      const permissions = (plugin.permissions ?? []).map((p) => `${p.key}=${p.status}`).join(", ")
      return `${disabled}${boxes} consent box(es); state ${plugin.summary.state}; permissions ${permissions}; ${consentShot}; ${await screenshot("enabled")}`
    },
  ],
  [
    "uninstall",
    async () => {
      await installedTab()
      await click(`${installedItem}${button("Uninstall")}`)
      await wait(DIALOG)
      const confirmShot = await screenshot("uninstall-confirm")
      await click(`${DIALOG}${button("Uninstall")}`)
      await dialogClosed()
      await until("the plugin to be gone", async () => {
        const list = await invoke("list_plugins", {})
        return Array.isArray(list) && list.every((p) => p.id !== PLUGIN)
      })
      const left = (await invoke("list_plugins", {}))
        .map((p) => `${p.id}@${p.version ?? "?"}`)
        .join(", ")
      return `gone from list_plugins (left: ${left || "none"}); ${confirmShot}; ${await screenshot("uninstalled")}`
    },
  ],
]

async function main() {
  try {
    const created = await call("POST", "/session", {
      capabilities: { alwaysMatch: { "tauri:options": { application: app } } },
    })
    session = created.sessionId
    if (!session) throw new Error(`no session id: ${JSON.stringify(created)}`)
    await call("POST", s("timeouts"), { script: 120_000, pageLoad: 120_000, implicit: 0 })
    record("WebDriver session on the release build", "pass", `session ${session} for ${app}`)
  } catch (e) {
    // Windows: msedgedriver hands WebView2 its debugging port through
    // WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, which WebView2 ignores when the
    // app passes browser arguments of its own (wry's defaults); only debug
    // builds merge the variable in (apps/desktop/src-tauri/src/windows.rs,
    // for_webdriver). The AdditionalBrowserArguments registry override is
    // ignored the same way (checked against 0.8.0).
    const why = /DevToolsActivePort/.test(e.message)
      ? " (release builds pass their own WebView2 browser arguments, so WebView2 ignores msedgedriver's WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS (--remote-debugging-port) and the webview never opens its debugging port; windows::for_webdriver, which merges it in, is debug-only)"
      : ""
    record(
      "WebDriver session on the release build",
      "not-covered",
      `the installed release build could not be driven: ${e.message}${why}`,
    )
    return
  }
  let failed = null
  for (const [name, run] of steps) {
    if (failed) {
      record(name, "not-covered", `not reached: '${failed}' failed`)
      continue
    }
    try {
      record(name, "pass", await run())
    } catch (e) {
      failed = name
      const shot = await screenshot(`failure-${name.replace(/[^a-z0-9]+/gi, "-")}`)
      try {
        writeFileSync(
          join(out, `${label}-gui-failure.html`),
          String(await call("GET", s("source"))),
        )
      } catch {}
      record(name, "fail", `${e.message}; ${shot}`)
    }
  }
  try {
    await call("DELETE", `/session/${session}`)
  } catch {}
}

await main()
