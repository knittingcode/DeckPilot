// Pure helpers of the marketplace screens.

import type { CatalogEntry, MarketplaceSnapshot, PluginCategory, RegistryPublisher } from "./types"

export function publisherName(publisher: RegistryPublisher | null, namespace: string): string {
  return publisher ? (publisher.display_name ?? publisher.login) : namespace
}

/** Why nothing can be installed or updated right now, or `null`. */
export function installBlock(snapshot: MarketplaceSnapshot): string | null {
  if (!snapshot.environment.pluginsSupported) return "Plugins are not available on this platform."
  switch (snapshot.registry.state) {
    case "fresh":
      return null
    case "offline":
      return "Installing and updating need a connection to the registry."
    case "stale":
      return "The registry is out of date; installing and updating are paused until it is checked again."
    case "verification-failed":
      return "The registry did not verify; installing and updating are paused."
    case "never-reached":
      return "The registry has not been reached yet."
  }
}

export const CATEGORY_LABELS: Record<PluginCategory, string> = {
  "developer-tools": "Developer tools",
  productivity: "Productivity",
  communication: "Communication",
  monitoring: "Monitoring",
  media: "Media",
  "smart-home": "Smart home",
  other: "Other",
}

export function categoryLabel(category: string): string {
  return Object.hasOwn(CATEGORY_LABELS, category)
    ? CATEGORY_LABELS[category as PluginCategory]
    : category
}

/** Entries whose name, id, summary, publisher or category contain every word. */
export function matchesQuery(item: CatalogEntry, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  const text = [
    item.entry.name,
    item.entry.id,
    item.entry.summary,
    publisherName(item.publisher, item.entry.publisher),
    ...(item.entry.categories ?? []).map(categoryLabel),
  ]
    .join(" ")
    .toLowerCase()
  return words.every((word) => text.includes(word))
}
