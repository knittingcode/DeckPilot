import {
  ArrowRightLeft,
  Database,
  Info,
  KeyRound,
  MonitorSmartphone,
  Palette,
  Puzzle,
  RefreshCw,
  ScrollText,
  Server,
  ShieldCheck,
  Variable,
  type LucideIcon,
} from "lucide-react"
import { lazy } from "react"

import type { ModuleInfo } from "@/lib/types"
import { moduleSettingsPages } from "@/modules"

type View = React.LazyExoticComponent<() => React.JSX.Element>

export type SettingsCategory = "general" | "hardware" | "security" | "extensions" | "advanced"

export const CATEGORIES: { id: SettingsCategory; label: string }[] = [
  { id: "general", label: "General" },
  { id: "hardware", label: "Hardware" },
  { id: "security", label: "Security & Data" },
  { id: "extensions", label: "Extensions" },
  { id: "advanced", label: "Advanced" },
]

export interface SettingsPageEntry {
  id: string
  title: string
  /** Shown under the page title. */
  description: string
  category: SettingsCategory
  icon: LucideIcon
  /** Further words the settings search matches. */
  keywords: string[]
  /** `null` for module pages generated from the manifest. */
  view: View | null
  /** Set for pages that belong to a module. */
  module?: ModuleInfo
}

const view = (load: () => Promise<{ default: () => React.JSX.Element }>) => lazy(load)

/**
 * Pages of the core application. Only settings the application actually
 * supports have a page; a category without pages is not shown.
 */
const CORE_PAGES: SettingsPageEntry[] = [
  {
    id: "appearance",
    title: "Appearance",
    description: "Theme of the configuration app",
    category: "general",
    icon: Palette,
    keywords: ["theme", "dark", "light", "mode"],
    view: view(() => import("./pages/appearance").then((m) => ({ default: m.AppearanceSettings }))),
  },
  {
    id: "updates",
    title: "Updates",
    description: "Signed updates from GitHub Releases",
    category: "general",
    icon: RefreshCw,
    keywords: ["version", "release", "download", "install", "upgrade"],
    view: view(() => import("./pages/updates").then((m) => ({ default: m.UpdatesSettings }))),
  },
  {
    id: "variables",
    title: "Shared Variables",
    description: "Values that actions and workflows read with {{ shared.… }}",
    category: "general",
    icon: Variable,
    keywords: ["variables", "shared", "values", "references", "secrets", "system"],
    view: view(() =>
      import("@/pages/shared-variables").then((m) => ({ default: m.SharedVariablesSettings })),
    ),
  },
  {
    id: "devices",
    title: "Devices",
    description: "Connected Stream Decks, brightness and gesture timing",
    category: "hardware",
    icon: MonitorSmartphone,
    keywords: [
      "stream deck",
      "hardware",
      "brightness",
      "connected",
      "usb",
      "gesture",
      "double click",
      "long press",
    ],
    view: view(() => import("./pages/devices").then((m) => ({ default: m.DevicesSettings }))),
  },
  {
    id: "agent",
    title: "Background Agent",
    description: "The process that drives your devices and runs actions",
    category: "hardware",
    icon: Server,
    keywords: ["agent", "autostart", "login", "start", "stop", "process"],
    view: view(() => import("./pages/agent").then((m) => ({ default: m.AgentSettings }))),
  },
  {
    id: "security",
    title: "Security",
    description: "Encryption, unlocking and automatic locking",
    category: "security",
    icon: ShieldCheck,
    keywords: ["password", "keychain", "lock", "touch id", "encryption", "authentication"],
    view: view(() => import("./pages/security").then((m) => ({ default: m.SecuritySettings }))),
  },
  {
    id: "backup",
    title: "Backup & Restore",
    description: "Encrypted export and import of your configuration",
    category: "security",
    icon: ArrowRightLeft,
    keywords: ["export", "import", "backup", "restore", "transfer", "migrate"],
    view: view(() => import("./pages/backup").then((m) => ({ default: m.BackupSettings }))),
  },
  {
    id: "storage",
    title: "Storage",
    description: "Where and how the configuration is stored",
    category: "security",
    icon: Database,
    keywords: ["database", "sqlcipher", "path", "data", "upgrade notes"],
    view: view(() => import("./pages/storage").then((m) => ({ default: m.StorageSettings }))),
  },
  {
    id: "modules",
    title: "Modules",
    description: "Features of DeckPilot you can enable or disable",
    category: "extensions",
    icon: Puzzle,
    keywords: ["plugins", "extensions", "features", "enable", "disable"],
    view: view(() => import("./pages/modules").then((m) => ({ default: m.ModulesSettings }))),
  },
  {
    id: "permissions",
    title: "Permissions",
    description: "What each module is allowed to access",
    category: "extensions",
    icon: KeyRound,
    keywords: ["capabilities", "access", "privacy"],
    view: view(() =>
      import("./pages/permissions").then((m) => ({ default: m.PermissionsSettings })),
    ),
  },
  {
    id: "diagnostics",
    title: "Logs & Diagnostics",
    description: "Logs and technical details for troubleshooting",
    category: "advanced",
    icon: ScrollText,
    keywords: ["log", "debug", "troubleshoot", "support", "ipc", "paths"],
    view: view(() =>
      import("./pages/diagnostics").then((m) => ({ default: m.DiagnosticsSettings })),
    ),
  },
  {
    id: "about",
    title: "About",
    description: "Version, license and system",
    category: "advanced",
    icon: Info,
    keywords: ["version", "license", "open source"],
    view: view(() => import("./pages/about").then((m) => ({ default: m.AboutSettings }))),
  },
]

/** Every settings page, with the pages of enabled modules under Extensions. */
export function settingsPages(modules: ModuleInfo[]): SettingsPageEntry[] {
  const fromModules = moduleSettingsPages(modules).map((page): SettingsPageEntry => ({
    id: page.id,
    title: page.module.name,
    description: `Settings of the ${page.module.name} module`,
    category: "extensions",
    icon: page.icon,
    keywords: ["module", page.module.id, ...page.module.config.map((spec) => spec.label)],
    view: page.view ?? null,
    module: page.module,
  }))
  return [...CORE_PAGES, ...fromModules]
}

export function findSettingsPage(modules: ModuleInfo[], id: string) {
  return settingsPages(modules).find((page) => page.id === id)
}

/** Pages grouped by category, in display order; empty categories are left out. */
export function groupedSettings(pages: SettingsPageEntry[]) {
  return CATEGORIES.map((category) => ({
    ...category,
    pages: pages.filter((page) => page.category === category.id),
  })).filter((group) => group.pages.length > 0)
}

/** Pages whose title, description or keywords contain every word of `query`. */
export function searchSettings(pages: SettingsPageEntry[], query: string) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return pages
  return pages.filter((page) => {
    const text = [page.title, page.description, ...page.keywords].join(" ").toLowerCase()
    return words.every((word) => text.includes(word))
  })
}
