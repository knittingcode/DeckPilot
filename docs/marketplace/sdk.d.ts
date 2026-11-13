/**
 * @deckpilot/plugin-sdk, plugin API level 1.0: reference declarations.
 *
 * DESIGN ARTIFACT, NOT A PUBLISHED PACKAGE. This file records the interface
 * that docs/marketplace/architecture.md (section 6) proposes. Implementation
 * phase P10 turns it into the real package; until then it is only checked by
 * the repository's lint and type checks, together with the example plugin.
 *
 * Rules that hold for every member:
 * - Metadata (ids, parameter schemas, outputs, variables, settings,
 *   permissions) lives in manifest.json. Code only binds handlers to ids
 *   declared there. The agent never runs plugin code to learn metadata.
 * - Every `ctx` call is a message to the agent, which checks it against the
 *   consent stored for this plugin. A call outside the consent rejects with
 *   `PluginError` code `not-permitted`. Members are always present so that
 *   the type does not depend on the grant; the grant is checked at run time.
 * - The runtime has no Node, DOM or Deno APIs and no ambient I/O (section 3).
 */

// ---------------------------------------------------------------------------
// Runtime globals (section 3.6 of the design). ECMAScript 2022 built-ins plus
// exactly these. There is no fetch, require, process, Buffer, WebSocket,
// Worker, WebAssembly, eval or Function constructor.
// ---------------------------------------------------------------------------

declare global {
  interface AbortSignal {
    readonly aborted: boolean
    readonly reason: unknown
    throwIfAborted(): void
    addEventListener(type: "abort", listener: () => void): void
    removeEventListener(type: "abort", listener: () => void): void
  }
  var AbortSignal: { timeout(ms: number): AbortSignal; any(signals: AbortSignal[]): AbortSignal }
  class AbortController {
    readonly signal: AbortSignal
    abort(reason?: unknown): void
  }
  class URLSearchParams {
    constructor(init?: string | Record<string, string>)
    get(name: string): string | null
    set(name: string, value: string): void
    append(name: string, value: string): void
    toString(): string
  }
  class URL {
    constructor(url: string, base?: string | URL)
    readonly searchParams: URLSearchParams
    href: string
    protocol: string
    host: string
    hostname: string
    pathname: string
    search: string
    toString(): string
  }
  class TextEncoder {
    encode(input: string): Uint8Array
  }
  class TextDecoder {
    constructor(label?: "utf-8")
    decode(input: Uint8Array): string
  }
  /** Routed to ctx.log of the current invocation. */
  var console: {
    debug(...data: unknown[]): void
    info(...data: unknown[]): void
    log(...data: unknown[]): void
    warn(...data: unknown[]): void
    error(...data: unknown[]): void
  }
  /** Bound to the current invocation: cleared when its promise settles or aborts. */
  function setTimeout(callback: () => void, ms: number): number
  function clearTimeout(id: number): void
  function queueMicrotask(callback: () => void): void
  function structuredClone<T>(value: T): T
  function atob(data: string): string
  function btoa(data: string): string
  var crypto: {
    getRandomValues<T extends Uint8Array | Uint32Array>(array: T): T
    randomUUID(): string
  }
}

/** Plugin API level this SDK implements. Compared with `engines.pluginApi`. */
export declare const API_VERSION: "1.0"

// ---------------------------------------------------------------------------
// Manifest-derived types
// ---------------------------------------------------------------------------

/** Value types a handler or published variable can carry. */
export type OutputValue = string | number | boolean

/** JSON value of a `json` variable or a storage entry. */
export type JsonValue =
  string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue }

/** The shape `deckpilot-plugin codegen` derives from manifest.json. */
export interface ManifestShape {
  /** Per action id: normalized parameter literals and declared outputs. */
  actions: Record<string, { params: object; outputs: Record<string, OutputValue> }>
  /** Per relative variable id: the TypeScript type of its value. */
  variables: Record<string, JsonValue>
  /** Normalized settings values. */
  settings: object
  /** Union of `contributes.credentials[].key`. */
  credentials: string
}

/**
 * Augmented by the generated `deckpilot-env.d.ts`. Without the generated
 * file, the generic `ManifestShape` is used and nothing is checked by key.
 */
export interface GeneratedManifest {
  /** Placeholder so the interface is never empty before augmentation. */
  readonly __generated?: never
}

/** The manifest type plugins compile against. */
export type Manifest = GeneratedManifest extends ManifestShape ? GeneratedManifest : ManifestShape

export type ActionIds<M extends ManifestShape> = keyof M["actions"] & string
export type ParamsOf<M extends ManifestShape, K extends ActionIds<M>> = M["actions"][K]["params"]
export type OutputsOf<M extends ManifestShape, K extends ActionIds<M>> = M["actions"][K]["outputs"]
export type VariableIds<M extends ManifestShape> = keyof M["variables"] & string
export type CredentialRefs<M extends ManifestShape> = M["credentials"]

/** A project id, as `ParameterValue::Project` carries it. */
export type ProjectId = string & { readonly __brand: "ProjectId" }

/** Same parsing as the core's `lines` and `pairs` kinds. */
export declare const text: {
  lines(value: string): string[]
  pairs(value: string, separator: "=" | ":"): [name: string, value: string][]
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/**
 * The single default export of plugin.js. The agent refuses a bundle whose
 * handler ids differ from the manifest's action ids (missing or extra).
 */
export declare function definePlugin<M extends ManifestShape = Manifest>(
  plugin: PluginDefinition<M>,
): PluginDefinition<M>

export interface PluginDefinition<M extends ManifestShape = Manifest> {
  /** One handler per `contributes.actions[].id`. */
  actions: { [K in ActionIds<M>]: ActionDefinition<M, K> }
  /**
   * Called when the plugin is enabled and after each agent start or reload.
   * 2 s budget; timers (permission `background`) are created here.
   */
  onActivate?(ctx: PluginContext<M>): Promise<void>
  /** Called before disable, uninstall, update or agent stop. 5 s budget. */
  onDeactivate?(ctx: PluginContext<M>): Promise<void>
}

export interface ActionDefinition<M extends ManifestShape, K extends ActionIds<M>> {
  /**
   * Runs the action. `params` are normalized literals: references and
   * templates were resolved by the agent, defaults filled, ranges checked.
   * Deadline: `node.timeoutMs` (default 30 s, at most 10 min).
   */
  run(ctx: ActionContext<M>, params: ParamsOf<M, K>): Promise<ActionResult<OutputsOf<M, K>>>
  /**
   * Live key state, pulled by the agent (ActionHandler::button_state). Only
   * called if the manifest declares `state` for this action. Must not change
   * anything; 20 s deadline (the agent's CALL_TIMEOUT for button states).
   */
  state?(ctx: StateContext<M>, params: ParamsOf<M, K>): Promise<ButtonState>
}

// ---------------------------------------------------------------------------
// Contexts
// ---------------------------------------------------------------------------

export interface HostInfo {
  /** DeckPilot version, e.g. "0.8.0". */
  readonly deckpilotVersion: string
  /** Negotiated API level: min(host level, bundle level), e.g. "1.0". */
  readonly apiVersion: string
  readonly platform: "macos" | "windows" | "linux"
  /** Feature detection for additions within API major 1. */
  has(feature: HostFeature): boolean
}

/** Optional additions announced in the handshake. Unknown names return false. */
export type HostFeature = "device.icon" | "crypto.hmac" | "http.stream"

export interface Disposable {
  dispose(): void
}

/** Members every context has. */
export interface BaseContext<M extends ManifestShape = Manifest> {
  readonly plugin: { readonly id: string; readonly version: string; readonly dev: boolean }
  readonly host: HostInfo
  /** Normalized settings (`contributes.settings`), read-only for the plugin. */
  readonly settings: Readonly<M["settings"]>
  /** Permission `network`. */
  readonly http: HttpApi<M>
  /** Permission `credentials`. */
  readonly credentials: CredentialsApi<M>
  /** Always available: 256 KiB by default, more with permission `storage`. */
  readonly storage: StorageApi
  /** Own variables always; others with permission `variables`. */
  readonly variables: VariablesApi<M>
  /** Permission `notifications`. */
  readonly notify: NotifyApi
  /** Always available; goes to the per-plugin log ring buffer. */
  readonly log: Logger
  /** Aborted on cancellation, on the deadline and on deactivation. */
  readonly signal: AbortSignal
}

export interface Invocation {
  readonly source: "key" | "workflow" | "trigger" | "app"
  readonly executionId?: string
  readonly workflowId?: string
  readonly nodeId?: string
  /** Absolute deadline, ms since the epoch. */
  readonly deadline: number
  /** 1-based; above 1 only for `retrySafe` nodes. */
  readonly attempt: number
}

export interface ActionContext<M extends ManifestShape = Manifest> extends BaseContext<M> {
  readonly invocation: Invocation
  /** Permission `profile` with read "profile"; otherwise undefined. */
  readonly profile: { readonly id: string; readonly name: string } | undefined
  /** Permission `profile` with read "project"; null when no project is active. */
  readonly project: ProjectInfo | null | undefined
  readonly device: DeviceApi
  /** ExecutionContext::report_progress; ignored outside workflows. */
  progress(percent: number | undefined, message?: string): void
  /** Resolves false if the context was aborted first. */
  sleep(ms: number): Promise<boolean>
  /**
   * Permission `openUrl`: https only, declared hosts only, only while the
   * invocation came from a key press or the app (a user gesture).
   */
  open(url: string): Promise<void>
}

export interface StateContext<M extends ManifestShape = Manifest> extends BaseContext<M> {
  readonly project: ProjectInfo | null | undefined
  readonly device: Pick<DeviceApi, "capabilities">
}

/** onActivate and onDeactivate: no invocation and no device. */
export interface PluginContext<M extends ManifestShape = Manifest> extends BaseContext<M> {
  /** Permission `background`. Cleared on deactivation. */
  readonly timers: TimersApi
  /** Called after the user saved settings; `ctx.settings` is already new. */
  onSettingsChanged(listener: () => void): Disposable
}

export interface ProjectInfo {
  readonly id: ProjectId
  readonly key: string
  readonly name: string
}

// ---------------------------------------------------------------------------
// HTTP and credentials
// ---------------------------------------------------------------------------

export interface HttpApi<M extends ManifestShape = Manifest> {
  /**
   * A fetch-shaped request performed by the agent's HTTP client.
   * - The host must match `permissions[network].hosts`; https only, except
   *   user-entered private hosts when `privateNetwork` is granted.
   * - Every resolved address is checked (NetworkPolicy); loopback, private,
   *   link-local, CGNAT, ULA and metadata ranges are refused unless the host
   *   is a user-entered private host. The connection goes to the checked
   *   address (no DNS rebinding).
   * - At most 5 redirects, each re-checked; never https to http; never to a
   *   host outside the list; credentials are dropped on a cross-origin hop.
   * - Body read up to `maxBytes` (default 1 MiB, max 10 MiB).
   * - Rejects with codes host-not-allowed, blocked-address, dns-failed,
   *   connection-failed, tls-failed, too-many-redirects, insecure-redirect,
   *   insecure-credentials, sensitive-header, invalid-url, request-failed,
   *   timeout, throttled. A non-2xx status is not an error.
   */
  fetch(url: string | URL, init?: HttpRequestInit<M>): Promise<HttpResponse>
}

export interface HttpRequestInit<M extends ManifestShape = Manifest> {
  method?: "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE"
  /** Secret-looking header names (core `looks_secret`) are refused; use `auth`. */
  headers?: Record<string, string>
  body?: string | Uint8Array | { json: JsonValue }
  /** Injects a stored secret inside the agent; plugin code never sees it. */
  auth?: CredentialAuth<M>
  /** At most the remaining deadline. */
  timeoutMs?: number
  maxBytes?: number
  /** Default true. */
  followRedirects?: boolean
  /** Combined with ctx.signal. */
  signal?: AbortSignal
}

export interface HttpResponse {
  readonly status: number
  readonly ok: boolean
  /** Final URL without query string. */
  readonly url: string
  /** Lower-case names. Set-Cookie and secret-looking headers read "[redacted]". */
  readonly headers: Readonly<Record<string, string>>
  readonly truncated: boolean
  readonly redirects: number
  text(): Promise<string>
  json(): Promise<JsonValue>
  bytes(): Promise<Uint8Array>
}

/**
 * Credentials are handles. The secret lives in the agent's per-plugin
 * credential table; it is injected into requests to the ref's own hosts and
 * is never returned to plugin code (API 1 has no reveal).
 */
export interface CredentialsApi<M extends ManifestShape = Manifest> {
  ref<K extends CredentialRefs<M>>(key: K): CredentialHandle<K>
  /** Whether the user stored a secret for this ref. Never returns the value. */
  has(key: CredentialRefs<M>): Promise<boolean>
}

/** Opaque; serializes to {"$credential": key} on the wire. */
export interface CredentialHandle<K extends string = string> {
  readonly key: K
  readonly __brand: "CredentialHandle"
}

/** Exactly the HTTP module's Auth::{Bearer, Basic, Header, Query}. */
export type CredentialAuth<M extends ManifestShape = Manifest> =
  | { credential: CredentialHandle<CredentialRefs<M>>; scheme: "bearer" }
  | { credential: CredentialHandle<CredentialRefs<M>>; scheme: "basic" }
  | { credential: CredentialHandle<CredentialRefs<M>>; scheme: "header" }
  | { credential: CredentialHandle<CredentialRefs<M>>; scheme: "query" }

// ---------------------------------------------------------------------------
// Variables
// ---------------------------------------------------------------------------

export interface VariablesApi<M extends ManifestShape = Manifest> {
  /**
   * Publishes the value of one of the plugin's own declared variables
   * (relative id; the agent adds `plugins.<publisher>.<name>.`). The value
   * must match `valueType`. `scope` must match the declaration:
   * machine: omitted; profile: { profileId }; project: { projectId }.
   * Not persisted; 10 publishes/s per plugin; value at most 16 KiB.
   */
  publish<K extends VariableIds<M>>(
    id: K,
    value: M["variables"][K] | null,
    scope?: { profileId?: string; projectId?: ProjectId },
  ): Promise<void>
  /** Clears published values of one variable or of all of them. */
  clear(id?: VariableIds<M>): Promise<void>
  /**
   * Reads a variable by canonical id. Own namespace always; others only if
   * listed in permissions[variables].read. Sensitive variables of other
   * owners and secret references resolve to permissionDenied.
   */
  get(id: string): Promise<VariableResolution>
}

export type VariableResolution =
  | { readonly status: "value"; readonly value: JsonValue; readonly sensitive: boolean }
  | {
      readonly status: "missing"
      readonly reason:
        "unknownVariable" | "noValue" | "noProject" | "noDevice" | "noWorkflow" | "noExecution"
    }
  | { readonly status: "unavailable"; readonly owner: string }
  | { readonly status: "permissionDenied" }
  | { readonly status: "unavailableUntilRuntime" }

// ---------------------------------------------------------------------------
// Device
// ---------------------------------------------------------------------------

export interface DeviceApi {
  /** Generic capabilities of the triggering device; null without one. */
  capabilities(): DeviceCapabilities | null
  /**
   * Permission `device.state`. Sets the live state of the keys bound to
   * this action instance only. Replaced by the next `state()` pull.
   */
  setState(state: ButtonState): Promise<void>
}

export type DeviceFamily = "keys" | "keyDisplays" | "dials" | "touchStrip" | "brightness"

export interface DeviceCapabilities {
  readonly families: readonly DeviceFamily[]
  readonly keys?: {
    readonly rows: number
    readonly columns: number
    readonly image?: { readonly width: number; readonly height: number }
  }
  readonly dials?: { readonly count: number; readonly pressable: boolean }
  readonly touchStrip?: { readonly image?: { readonly width: number; readonly height: number } }
}

/** The core ButtonState. Texts are sanitized to 32 characters. */
export interface ButtonState {
  label?: string
  detail?: string
  indicator?: "ok" | "warning" | "error" | "running" | "unknown" | "loading"
  /** 0 to 100. */
  progress?: number
  toggle?: boolean
  /**
   * API 1.1 (`host.has("device.icon")`), permission `device.icons`: the
   * name of a PNG declared in `assets.keyIcons`.
   */
  icon?: string
}

// ---------------------------------------------------------------------------
// Storage, notifications, log, timers
// ---------------------------------------------------------------------------

/** Per-plugin, per-computer key-value store in the encrypted database. Never exported. */
export interface StorageApi {
  get(key: string): Promise<JsonValue | undefined>
  /** Key at most 128 characters; value at most 64 KiB of JSON. */
  set(key: string, value: JsonValue): Promise<void>
  delete(key: string): Promise<void>
  keys(prefix?: string): Promise<string[]>
  /** Exceeding the quota rejects with `storage-quota`. */
  usage(): Promise<{ used: number; quota: number }>
}

export interface Notification {
  title: string
  body?: string
  severity?: "info" | "success" | "warning" | "error"
  desktop?: boolean
  statusBar?: boolean
}

/** Shown attributed to the plugin by name; at most 5 per minute. */
export type NotifyApi = (notification: Notification) => Promise<"shown" | "suppressed">

export type LogFields = Record<string, string | number | boolean>

/**
 * Per-plugin log (ring buffer, shown in Settings > Plugins > Activity).
 * Lines are prefixed by the agent, at most 2 KiB each, 50 per second, then
 * sampled. Known secrets are replaced by [redacted].
 */
export interface Logger {
  debug(message: string, fields?: LogFields): void
  info(message: string, fields?: LogFields): void
  warn(message: string, fields?: LogFields): void
  error(message: string, fields?: LogFields): void
}

/**
 * Permission `background`. Interval at least minIntervalSeconds (>= 15 s);
 * at most 16 live timers; each callback gets 30 s and its own signal.
 */
export interface TimersApi {
  every(ms: number, callback: (signal: AbortSignal) => Promise<void>): Disposable
  after(ms: number, callback: (signal: AbortSignal) => Promise<void>): Disposable
}

// ---------------------------------------------------------------------------
// Results and errors
// ---------------------------------------------------------------------------

export interface ActionResult<O extends Record<string, OutputValue> = Record<string, OutputValue>> {
  readonly status: "succeeded" | "failed" | "skipped"
  /** At most 500 characters after the agent shortens it. Never put secrets in it. */
  readonly message: string
  /** Only declared outputs are kept; the agent stringifies them per kind. */
  readonly data?: Partial<O>
  /** Required when failed: a code from contributes.errorCodes or a SharedCode. */
  readonly code?: string
}

export declare function ok<O extends Record<string, OutputValue>>(
  message: string,
  data?: O,
): ActionResult<O>
export declare function skipped(message: string): ActionResult<never>
export declare function failed(
  code: string,
  message: string,
  data?: Record<string, OutputValue>,
): ActionResult<never>

/** Throw to fail with a code; the same as returning failed(code, message). */
export declare class PluginError extends Error {
  constructor(
    code: string,
    message: string,
    options?: { data?: Record<string, OutputValue>; retryable?: boolean },
  )
  readonly code: string
  readonly data?: Record<string, OutputValue>
  readonly retryable: boolean
}

/** Codes the agent produces; plugins may return them too. */
export type SharedCode =
  // existing core codes
  | "not-permitted"
  | "invalid-parameters"
  | "no-active-project"
  | "credential-missing"
  | "reference-missing"
  | "reference-type"
  | "reference-denied"
  // existing HTTP module codes, reused by ctx.http
  | "blocked-address"
  | "dns-failed"
  | "connection-failed"
  | "tls-failed"
  | "too-many-redirects"
  | "insecure-redirect"
  | "insecure-credentials"
  | "request-failed"
  | "sensitive-header"
  | "invalid-url"
  // new for plugins
  | "host-not-allowed"
  | "throttled"
  | "storage-quota"
  | "timeout"
  | "unsupported"
  | "plugin-error"
  | "plugin-unavailable"
  | "invalid-result"
