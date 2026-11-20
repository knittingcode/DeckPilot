// The words of the consent to a server the user entered (G2).

import { i18n } from "@/i18n"

import { listed } from "./model"
import type { PluginUserOrigin } from "./types"

export { listed }

/** `https://server[:port]` of an origin with a path prefix. */
function serverOf(origin: string | undefined): string | undefined {
  if (!origin) return undefined
  const slash = origin.indexOf("/", 8)
  return slash < 0 ? origin : origin.slice(0, slash)
}

/** The names of secrets in quotes, as a list: “Token” and “Key”. */
export function quotedList(names: string[]): string {
  return listed(names.map((name) => i18n.t("plugins:origins.quoted", { name })))
}

/** What the consent allows, in words. */
export function consentText(plugin: string, origin: PluginUserOrigin): string {
  const server = origin.origin ?? i18n.t("plugins:origins.consent.thisServer")
  const credentials = origin.credentials ?? []
  const ask =
    credentials.length > 0
      ? i18n.t("plugins:origins.consent.askWithSecrets", {
          plugin,
          server,
          secrets: quotedList(credentials),
          count: credentials.length,
        })
      : i18n.t("plugins:origins.consent.ask", { plugin, server })
  const requests = i18n.t(
    origin.origin?.includes("/", 8)
      ? "plugins:origins.consent.requestsPath"
      : "plugins:origins.consent.requests",
    { methods: origin.methods.join(", ") },
  )
  const pages = origin.opensPages
    ? i18n.t("plugins:origins.consent.pages", {
        server: serverOf(origin.origin) ?? i18n.t("plugins:origins.consent.thisServer"),
      })
    : null
  return [ask, requests, pages].filter(Boolean).join(" ")
}
