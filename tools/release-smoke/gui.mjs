// Drives the INSTALLED release app through tauri-driver (W3C WebDriver over
// HTTP, Node's built-in fetch, no dependencies) against the production
// plugin Marketplace:
//
//   node tools/release-smoke/gui.mjs --port <tauri-driver port> --app <executable> --label <label> --out <dir>
//
// Main window › Settings › Plugins › Discover › Memory (deckpilot.memory)
// › Install with consent › Installed › Enable with consent › Uninstall.
// The selectors are those of apps/desktop/src-tauri/tests/marketplace_gui.rs.
// Each step is recorded in <out>/results.tsv (see common.sh); a step that
// fails stops the flow, the rest are recorded as not reached. A session
// that cannot be created is recorded as not covered, with the driver's error.
import { appendFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const args = Object.fromEntries(
  process.argv.slice(2).reduce((pairs, arg, i, all) => {
    if (arg.startsWith("--")) pairs.push([arg.slice(2), all[i + 1]]);
    return pairs;
  }, []),
);
const { port, app, label, out } = args;
const platform = process.env.SMOKE_PLATFORM ?? "unknown";
const ELEMENT = "element-6066-11e4-a52e-4f735466cecf";
const PLUGIN = "deckpilot.memory";
const NAME = "Memory";
const VERSION = "0.1.0";
const WAIT = 30_000;
const LONG = 120_000;
let shots = 0;

function record(step, result, evidence) {
  const clean = String(evidence ?? "").replace(/[\t\r\n]+/g, " ").slice(0, 900);
  appendFileSync(join(out, "results.tsv"), `${platform}\t${label}: GUI ${step}\t${result}\t${clean}\n`);
  console.log(`${result.toUpperCase()}  GUI ${step}: ${clean}`);
}

async function call(method, path, body) {
  const response = await fetch(`http://127.0.0.1:${port}${path}`, {
    method,
    headers: body ? { "content-type": "application/json" } : {},
    body: body ? JSON.stringify(body) : undefined,
    signal: AbortSignal.timeout(180_000),
  });
  const text = await response.text();
  let answer;
  try {
    answer = JSON.parse(text);
  } catch {
    throw new Error(`invalid answer (${response.status}): ${text.slice(0, 300)}`);
  }
  const value = answer.value;
  if (value && typeof value === "object" && value.error) {
    throw new Error(`${value.error}: ${value.message}`);
  }
  return value;
}

let session;
const s = (path) => `/session/${session}/${path}`;
const execute = (script, a = []) => call("POST", s("execute/sync"), { script, args: a });
const executeAsync = (script, a = []) => call("POST", s("execute/async"), { script, args: a });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const lit = (t) => (t.includes("'") ? `"${t}"` : `'${t}'`);
const button = (t) => `//button[normalize-space(.)=${lit(t)}]`;
const tab = (t) => `//button[@role='tab' and starts-with(normalize-space(.),${lit(t)})]`;
const DIALOG = "//*[@role='dialog' or @role='alertdialog']";

async function findAll(xpath) {
  const found = await call("POST", s("elements"), { using: "xpath", value: xpath });
  return (found ?? []).map((e) => e[ELEMENT]);
}

async function displayed(id) {
  return execute(
    "const e = arguments[0]; const r = e.getBoundingClientRect(); const st = getComputedStyle(e); " +
      "return r.width > 0 && r.height > 0 && st.visibility !== 'hidden' && st.display !== 'none';",
    [{ [ELEMENT]: id }],
  );
}

async function wait(xpath, timeout = WAIT) {
  const deadline = Date.now() + timeout;
  let last;
  for (;;) {
    try {
      for (const id of await findAll(xpath)) if (await displayed(id)) return id;
    } catch (e) {
      last = e;
    }
    if (Date.now() > deadline) {
      throw new Error(`no displayed element for ${xpath} within ${timeout / 1000} s${last ? ` (last error: ${last.message})` : ""}`);
    }
    await sleep(250);
  }
}

async function visible(xpath) {
  for (const id of await findAll(xpath).catch(() => [])) if (await displayed(id).catch(() => false)) return true;
  return false;
}

async function click(xpath, timeout = WAIT) {
  const deadline = Date.now() + timeout;
  for (;;) {
    const id = await wait(xpath, Math.max(1000, deadline - Date.now()));
    await execute("arguments[0].scrollIntoView({block: 'center', inline: 'center'});", [{ [ELEMENT]: id }]).catch(() => {});
    try {
      await call("POST", s(`element/${id}/click`), {});
      return;
    } catch (e) {
      if (Date.now() > deadline) throw e;
      await sleep(300);
    }
  }
}

async function text(xpath) {
  const id = await wait(xpath);
  return call("GET", s(`element/${id}/text`));
}

async function invoke(command, a = {}) {
  const answer = await executeAsync(
    "const done = arguments[arguments.length - 1]; window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1]).then(" +
      "(v) => done({ok: v === undefined ? null : v}), (e) => done({err: e === undefined ? null : e}))",
    [command, a],
  );
  if (answer && "err" in answer) throw new Error(`${command}: ${JSON.stringify(answer.err)}`);
  return answer?.ok;
}

async function until(what, f, timeout = LONG) {
  const deadline = Date.now() + timeout;
  let last;
  for (;;) {
    try {
      const v = await f();
      if (v) return v;
    } catch (e) {
      last = e;
    }
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}${last ? ` (last error: ${last.message})` : ""}`);
    await sleep(500);
  }
}

async function screenshot(name) {
  shots += 1;
  const file = `${label}-gui-${String(shots).padStart(2, "0")}-${name}.png`;
  try {
    const data = await call("GET", s("screenshot"));
    writeFileSync(join(out, file), Buffer.from(data, "base64"));
    return file;
  } catch (e) {
    console.log(`screenshot ${name} failed: ${e.message}`);
    return `(screenshot failed: ${e.message})`;
  }
}

async function consentAll() {
  const unchecked = `${DIALOG}//button[@role='checkbox' and @aria-checked='false']`;
  let n = 0;
  while ((await findAll(unchecked)).length > 0) {
    await click(unchecked);
    n += 1;
    if (n > 10) throw new Error("the consent boxes do not stay checked");
  }
  return n;
}

async function dialogClosed() {
  await until("the dialog to close", async () => !(await visible(DIALOG)), WAIT);
}

async function installedTab() {
  if (!(await visible(tab("Installed")))) {
    if (!(await visible(button("Plugins")))) await click(button("Settings"));
    await click(button("Plugins"));
  }
  await click(tab("Installed"));
  await wait("//ul[@aria-label='Installed plugins']");
}

const card = `//ul[@aria-label='Plugins']//button[.//span[normalize-space(.)=${lit(NAME)}]]`;

const steps = [
  ["main window", async () => {
    await wait(button("Settings"), LONG);
    const status = await invoke("agent_status", {});
    return `main window shown (button Settings); agent_status: ${JSON.stringify(status).slice(0, 300)}; ${await screenshot("main")}`;
  }],
  ["Settings > Extensions > Plugins", async () => {
    await click(button("Settings"));
    await click(button("Plugins"));
    await wait(tab("Discover"));
    return `Discover tab shown; ${await screenshot("plugins")}`;
  }],
  ["production catalog lists Memory", async () => {
    await click(tab("Discover"));
    try {
      await wait(card, 15_000);
    } catch {
      await click(button("Refresh"));
      await wait(card, LONG);
    }
    const available = await invoke("list_available_plugins", { refresh: false });
    const registry = available?.registry ?? {};
    const ids = (available?.plugins ?? []).map((p) => p.id);
    if (!ids.includes(PLUGIN)) throw new Error(`${PLUGIN} not in list_available_plugins: ${ids.join(", ")}`);
    return `card '${NAME}' shown; registry ${registry.source} seq ${registry.seq} freshness ${registry.freshness}; plugins: ${ids.join(", ")}; ${await screenshot("catalog")}`;
  }],
  [`install ${PLUGIN} ${VERSION} with consent`, async () => {
    await click(card);
    const title = await text("//h3[@id='plugin-detail-name']");
    if (title.trim() !== NAME) throw new Error(`details show '${title}'`);
    await click(button(`Install ${VERSION}`));
    await wait(DIALOG);
    const consentShot = await screenshot("install-consent");
    const boxes = await consentAll();
    await click(`${DIALOG}${button("Install")}`);
    await dialogClosed();
    const status = await until(`Installed ${VERSION}`, async () => {
      const t = await text("//section[@aria-label='Install status']");
      return t.includes(`Installed ${VERSION}`) ? t : null;
    });
    const plugin = await invoke("get_plugin", { plugin: PLUGIN });
    return `${boxes} consent box(es) checked; status '${status.trim()}'; get_plugin: version ${plugin?.summary?.version}, state ${plugin?.summary?.state}; ${consentShot}; ${await screenshot("installed")}`;
  }],
  ["enable with consent", async () => {
    await installedTab();
    await click(`//button[@role='switch' and @aria-label=${lit(`Enable ${NAME}`)}]`);
    await wait(DIALOG);
    const consentShot = await screenshot("enable-consent");
    const boxes = await consentAll();
    await click(`${DIALOG}${button("Enable")}`);
    await dialogClosed();
    await wait(`//button[@role='switch' and @aria-label=${lit(`Disable ${NAME}`)}]`, LONG);
    const plugin = await until("state enabled", async () => {
      const p = await invoke("get_plugin", { plugin: PLUGIN });
      return p?.summary?.state === "enabled" ? p : null;
    });
    const permissions = (plugin.permissions ?? []).map((p) => `${p.key}=${p.status}`).join(", ");
    return `${boxes} consent box(es); state ${plugin.summary.state}; permissions ${permissions}; ${consentShot}; ${await screenshot("enabled")}`;
  }],
  ["uninstall", async () => {
    await installedTab();
    await click(`//ul[@aria-label='Installed plugins']${button("Uninstall")}`);
    await wait(DIALOG);
    const confirmShot = await screenshot("uninstall-confirm");
    await click(`${DIALOG}${button("Uninstall")}`);
    await dialogClosed();
    await until("the plugin to be gone", async () => {
      const list = await invoke("list_plugins", {});
      return Array.isArray(list) && list.every((p) => p.id !== PLUGIN);
    });
    const left = (await invoke("list_plugins", {})).map((p) => `${p.id}@${p.version ?? "?"}`).join(", ");
    return `gone from list_plugins (left: ${left || "none"}); ${confirmShot}; ${await screenshot("uninstalled")}`;
  }],
];

async function main() {
  try {
    const created = await call("POST", "/session", {
      capabilities: { alwaysMatch: { "tauri:options": { application: app } } },
    });
    session = created.sessionId;
    if (!session) throw new Error(`no session id: ${JSON.stringify(created)}`);
    await call("POST", s("timeouts"), { script: 120_000, pageLoad: 120_000, implicit: 0 });
    record("WebDriver session on the release build", "pass", `session ${session} for ${app}`);
  } catch (e) {
    record("WebDriver session on the release build", "not-covered", `the installed release build could not be driven: ${e.message}`);
    return;
  }
  let failed = null;
  for (const [name, run] of steps) {
    if (failed) {
      record(name, "not-covered", `not reached: '${failed}' failed`);
      continue;
    }
    try {
      record(name, "pass", await run());
    } catch (e) {
      failed = name;
      const shot = await screenshot(`failure-${name.replace(/[^a-z0-9]+/gi, "-")}`);
      try {
        writeFileSync(join(out, `${label}-gui-failure.html`), String(await call("GET", s("source"))));
      } catch {}
      record(name, "fail", `${e.message}; ${shot}`);
    }
  }
  try {
    await call("DELETE", `/session/${session}`);
  } catch {}
}

await main();
