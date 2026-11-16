// The words of the consent to a server the user entered (G2).

import type { PluginUserOrigin } from "./types"

/** "A, B and C". */
export function listed(items: string[]): string {
  if (items.length <= 1) return items.join("")
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1] ?? ""}`
}

/** `https://server[:port]` of an origin with a path prefix. */
function serverOf(origin: string | undefined): string | undefined {
  if (!origin) return undefined
  const slash = origin.indexOf("/", 8)
  return slash < 0 ? origin : origin.slice(0, slash)
}

/** What the consent allows, in words. */
export function consentText(plugin: string, origin: PluginUserOrigin): string {
  const where = origin.origin ?? "this server"
  const secrets = origin.credentials?.length
    ? ` and your ${listed(origin.credentials.map((c) => `“${c}”`))} ${origin.credentials.length === 1 ? "secret" : "secrets"}`
    : ""
  const pages = origin.opensPages
    ? ` It may also open pages of ${serverOf(origin.origin) ?? "this server"} in your browser, in actions you start.`
    : ""
  return `Allow ${plugin} to send data${secrets} to ${where}? Requests (${origin.methods.join(", ")}) go to exactly this server${origin.origin?.includes("/", 8) ? " and path" : ""}, nowhere else.${pages}`
}
