#!/usr/bin/env node
/**
 * Live preview check of the Device Editor against the RUNNING app of a
 * development session (`tauri dev`) and its agent with SIMULATED devices
 * (`--mock-devices`): never a physical Stream Deck. The editor is driven
 * through the layout probe of the Vite dev server (dev/layout-probe-plugin.ts),
 * the simulated device through the agent's own IPC (`simulateInput` presses,
 * `inspectMockDevice` frames and navigation; both refused by an agent
 * without mock devices). See docs/editor-navigation-and-simulation.md,
 * section 3, and docs/development.md, "Layout probe".
 *
 *   DECKPILOT_VITE_PORT=1441 DECKPILOT_DEV_DIR=… DECKPILOT_DATA_DIR=… \
 *     node apps/desktop/scripts/live-check.mjs [--serial SIM-MK2-01]
 *
 * The main window must show the Device Editor for the simulated MK.2, on a
 * layout of the active profile whose home page binds key 1 (index 0) and
 * has a folder key (found by the check). The check:
 * - the editor reports "live";
 * - clearing a key's assignment (unsaved) redraws that key on the device
 *   and no other key;
 * - Undo restores the device's frame; Revert likewise;
 * - a double-click on a folder key moves the device into the folder;
 *   pressing the device's protected Back (index 0) brings the editor back;
 * - nothing unsaved is left: the editor shows "All changes saved".
 * Exit code 1 with the failures listed, 0 when all pass.
 */
import { readFileSync } from "node:fs"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const here = path.dirname(fileURLToPath(import.meta.url))
const port = Number(process.env.DECKPILOT_VITE_PORT ?? 1420)
const devDir = process.env.DECKPILOT_DEV_DIR ?? path.resolve(here, "../../../.dev")
const dataDir = process.env.DECKPILOT_DATA_DIR ?? path.join(devDir, "data")
const serialArg = process.argv.indexOf("--serial")
const serial = serialArg > 0 ? process.argv[serialArg + 1] : null
const probeToken = readFileSync(path.join(devDir, "layout-probe.token"), "utf8").trim()
const agentToken = readFileSync(path.join(dataDir, "agent.token"), "utf8").trim()

/** The agent's socket, as `deckpilot_ipc::Endpoint::for_data_dir` derives it. */
function socketPath() {
  if (process.env.DECKPILOT_AGENT_SOCKET) return process.env.DECKPILOT_AGENT_SOCKET
  let hash = 0xcbf29ce484222325n
  for (const byte of Buffer.from(dataDir)) {
    hash ^= BigInt(byte)
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn
  }
  const file = `agent-${hash.toString(16).padStart(16, "0")}.sock`
  const dir = `deckpilot-${os.userInfo().uid}`
  const runtime = process.env.XDG_RUNTIME_DIR
  const base = runtime && path.isAbsolute(runtime) ? runtime : os.tmpdir()
  const preferred = path.join(base, dir, file)
  const capacity = process.platform === "linux" ? 108 : 104
  return Buffer.byteLength(preferred) < capacity ? preferred : path.join("/tmp", dir, file)
}

/** One request on a connection of its own (protocol 16). */
function agent(request) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath())
    let buffer = ""
    let welcomed = false
    socket.setEncoding("utf8")
    socket.on("error", reject)
    socket.on("connect", () => {
      socket.write(
        `${JSON.stringify({ type: "hello", protocol: 16, token: agentToken, client: "other" })}\n`,
      )
    })
    socket.on("data", (chunk) => {
      buffer += chunk
      let newline
      while ((newline = buffer.indexOf("\n")) >= 0) {
        const message = JSON.parse(buffer.slice(0, newline))
        buffer = buffer.slice(newline + 1)
        if (!welcomed) {
          if (message.type !== "welcome") {
            reject(new Error(JSON.stringify(message)))
            socket.end()
            return
          }
          welcomed = true
          socket.write(`${JSON.stringify({ type: "request", id: 1, request })}\n`)
        } else if (message.id === 1) {
          socket.end()
          if (message.type === "error") reject(new Error(message.error.message))
          else resolve(message.response)
        }
      }
    })
  })
}

async function probe(command, args = {}) {
  for (let attempt = 0; ; attempt++) {
    const response = await fetch(`http://localhost:${port}/__deckpilot/probe`, {
      method: "POST",
      headers: { "x-deckpilot-probe": probeToken, "content-type": "application/json" },
      body: JSON.stringify({ command, args }),
    })
    if (response.status === 504 && attempt < 3) {
      await new Promise((r) => setTimeout(r, 1000))
      continue
    }
    const body = await response.json()
    if (!response.ok || body?.error)
      throw new Error(`${command}: ${body?.error ?? response.status}`)
    return body
  }
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms))
const failures = []
let checks = 0
function expect(condition, message) {
  checks++
  if (!condition) failures.push(message)
}

async function device() {
  const status = await agent({ type: "getStatus" })
  const mock = status.status.devices.filter(
    (d) => d.source === "mock" && d.state === "connected" && (!serial || d.id.serial === serial),
  )
  if (mock.length === 0) throw new Error("no simulated device is connected (--mock-devices)")
  return mock[0]
}

async function inspect(id) {
  return (await agent({ type: "inspectMockDevice", device: id })).inspection
}

const hashes = (inspection) =>
  Object.fromEntries(inspection.frames.map((f) => [`${f.control.kind}:${f.control.index}`, f.hash]))
const writes = (inspection) =>
  Object.fromEntries(
    inspection.frames.map((f) => [`${f.control.kind}:${f.control.index}`, f.writes]),
  )

/** Waits until `predicate` holds for the device, at most `ms`. */
async function until(id, predicate, ms = 3000) {
  const deadline = Date.now() + ms
  let last
  for (;;) {
    last = await inspect(id)
    if (predicate(last) || Date.now() > deadline) return last
    await sleep(100)
  }
}

async function main() {
  const shown = await device()
  const id = shown.id
  const live = await probe("live")
  expect(live.live === "live", `the editor's live preview is ${live.live} (${live.text})`)
  const start = await inspect(id)
  expect(start.navigation.inScope, "the device shows the edited layout")

  // An unsaved edit: key 1 (index 0) loses its assignment.
  await probe("select", { control: "key:0" })
  const before = await inspect(id)
  await probe("edit", { to: "clear" })
  const cleared = await until(id, (i) => hashes(i)["key:0"] !== hashes(before)["key:0"])
  expect(hashes(cleared)["key:0"] !== hashes(before)["key:0"], "the device redraws the edited key")
  for (const [key, count] of Object.entries(writes(cleared))) {
    if (key !== "key:0")
      expect(
        count === writes(before)[key],
        `${key} was drawn again (${writes(before)[key]} → ${count})`,
      )
  }
  expect((await probe("live")).unsaved !== null, "the editor shows the edit as unsaved")

  // Undo, then the same edit and Revert: the saved frame again.
  await probe("edit", { to: "undo" })
  const undone = await until(id, (i) => hashes(i)["key:0"] === hashes(before)["key:0"])
  expect(hashes(undone)["key:0"] === hashes(before)["key:0"], "undo restores the device's key")
  await probe("select", { control: "key:0" })
  await probe("edit", { to: "clear" })
  await until(id, (i) => hashes(i)["key:0"] !== hashes(before)["key:0"])
  await probe("edit", { to: "revert" })
  const reverted = await until(id, (i) => hashes(i)["key:0"] === hashes(before)["key:0"])
  expect(hashes(reverted)["key:0"] === hashes(before)["key:0"], "revert restores the device's key")

  // Editor -> device: a double-click on each key until one opens a folder.
  const keys = (await probe("keys")).filter((k) => k?.startsWith("key:"))
  let entered = null
  for (const key of keys) {
    const pageBefore = (await probe("live")).page
    await probe("dblclick", { control: key })
    const after = await probe("live")
    if (after.page !== pageBefore) {
      entered = { key, page: after.page }
      break
    }
  }
  if (!entered) {
    failures.push("no key of the home page navigates in the editor (bind a folder key)")
  } else {
    const inside = await until(id, (i) => i.navigation.stack.length > 1)
    expect(
      inside.navigation.stack.length > 1,
      `the device followed the editor into ${entered.page}`,
    )
    // Device -> editor: the protected Back (index 0) of the folder.
    for (const input of ["press", "release"]) {
      await agent({
        type: "simulateInput",
        device: id,
        control: { kind: "key", index: 0 },
        input: { type: input },
      })
    }
    await sleep(800)
    const back = await probe("live")
    expect(back.page !== entered.page, `the editor followed the device's Back (still ${back.page})`)
    const home = await inspect(id)
    expect(home.navigation.stack.length === 1, "the device is back on a top-level page")
  }
  const final = await probe("live")
  expect(
    final.unsaved !== null && !/unsaved/i.test(final.unsaved),
    `the editor ends ${final.unsaved}`,
  )
}

main()
  .catch((error) => {
    failures.push(String(error?.message ?? error))
  })
  .finally(() => {
    if (failures.length > 0) {
      console.error(`live-check: ${failures.length} of ${checks} checks failed`)
      for (const failure of failures) console.error(`  - ${failure}`)
      process.exit(1)
    }
    console.log(`live-check: ${checks} checks passed`)
  })
