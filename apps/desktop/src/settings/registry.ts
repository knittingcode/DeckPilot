import {
  AppWindow,
  ArrowRightLeft,
  Database,
  FolderOpen,
  Info,
  KeyRound,
  Languages,
  MonitorSmartphone,
  Puzzle,
  RefreshCw,
  ScrollText,
  Server,
  ShieldCheck,
  Store,
  UserRound,
  Variable,
  type LucideIcon,
} from "lucide-react"
import { lazy } from "react"

import { type TranslationKey, i18n } from "@/i18n"
import type { ModuleInfo } from "@/lib/types"
import { moduleSettingsPages } from "@/modules"

type View = React.LazyExoticComponent<() => React.JSX.Element>

export type SettingsCategory = "general" | "hardware" | "security" | "extensions" | "advanced"

const CATEGORY_LABELS = {
  general: "settings:categories.general",
  hardware: "settings:categories.hardware",
  security: "settings:categories.security",
  extensions: "settings:categories.extensions",
  advanced: "settings:categories.advanced",
} as const satisfies Record<SettingsCategory, TranslationKey>

const CATEGORY_ORDER: SettingsCategory[] = [
  "general",
  "hardware",
  "security",
  "extensions",
  "advanced",
]

/** The categories in display order, named in the window's language. */
export function settingsCategories(): { id: SettingsCategory; label: string }[] {
  return CATEGORY_ORDER.map((id) => ({ id, label: i18n.t(CATEGORY_LABELS[id]) }))
}

export function categoryLabel(id: SettingsCategory): string {
  return i18n.t(CATEGORY_LABELS[id])
}

/**
 * A page as the registry defines it: its texts are translation keys,
 * resolved in the window's language when the pages are listed
 * (`settingsPages`), so a change of language shows at the next render.
 */
interface PageDefinition extends Omit<SettingsPageEntry, "title" | "description" | "module"> {
  title: TranslationKey
  description: TranslationKey
}

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
const CORE_PAGES: PageDefinition[] = [
  {
    id: "language",
    title: "settings:pages.language.title",
    description: "settings:pages.language.description",
    category: "general",
    icon: Languages,
    keywords: ["language", "locale", "translation", "english", "polish", "polski", "język"],
    view: view(() => import("./pages/language").then((m) => ({ default: m.LanguageSettings }))),
  },
  {
    id: "profiles",
    title: "settings:pages.profiles.title",
    description: "settings:pages.profiles.description",
    category: "general",
    icon: UserRound,
    keywords: ["profile", "create", "rename", "duplicate", "delete", "active", "activate"],
    view: view(() => import("@/pages/profiles").then((m) => ({ default: m.ProfilesSettings }))),
  },
  {
    id: "updates",
    title: "settings:pages.updates.title",
    description: "settings:pages.updates.description",
    category: "general",
    icon: RefreshCw,
    keywords: ["version", "release", "download", "install", "upgrade"],
    view: view(() => import("./pages/updates").then((m) => ({ default: m.UpdatesSettings }))),
  },
  {
    id: "variables",
    title: "settings:pages.variables.title",
    description: "settings:pages.variables.description",
    category: "general",
    icon: Variable,
    keywords: [
      "variables",
      "shared",
      "values",
      "references",
      "secrets",
      "system",
      "module",
      "owner",
    ],
    view: view(() =>
      import("@/pages/shared-variables").then((m) => ({ default: m.SharedVariablesSettings })),
    ),
  },
  {
    id: "resources",
    title: "settings:pages.resources.title",
    description: "settings:pages.resources.description",
    category: "general",
    icon: FolderOpen,
    keywords: [
      "resources",
      "bindings",
      "folder",
      "directory",
      "application",
      "terminal",
      "url",
      "path",
      "machine",
      "computer",
      "zasoby",
    ],
    view: view(() => import("./pages/resources").then((m) => ({ default: m.ResourcesSettings }))),
  },
  {
    id: "devices",
    title: "settings:pages.devices.title",
    description: "settings:pages.devices.description",
    category: "hardware",
    icon: MonitorSmartphone,
    keywords: [
      "stream deck",
      "hardware",
      "brightness",
      "connected",
      "usb",
      "models",
      "device models",
      "supported",
      "gesture",
      "double click",
      "long press",
    ],
    view: view(() => import("./pages/devices").then((m) => ({ default: m.DevicesSettings }))),
  },
  {
    id: "agent",
    title: "settings:pages.agent.title",
    description: "settings:pages.agent.description",
    category: "hardware",
    icon: Server,
    keywords: ["agent", "autostart", "login", "start", "stop", "process"],
    view: view(() => import("./pages/agent").then((m) => ({ default: m.AgentSettings }))),
  },
  {
    id: "security",
    title: "settings:pages.security.title",
    description: "settings:pages.security.description",
    category: "security",
    icon: ShieldCheck,
    keywords: ["password", "keychain", "lock", "touch id", "encryption", "authentication"],
    view: view(() => import("./pages/security").then((m) => ({ default: m.SecuritySettings }))),
  },
  {
    id: "backup",
    title: "settings:pages.backup.title",
    description: "settings:pages.backup.description",
    category: "security",
    icon: ArrowRightLeft,
    keywords: ["export", "import", "backup", "restore", "transfer", "migrate"],
    view: view(() => import("./pages/backup").then((m) => ({ default: m.BackupSettings }))),
  },
  {
    id: "storage",
    title: "settings:pages.storage.title",
    description: "settings:pages.storage.description",
    category: "security",
    icon: Database,
    keywords: ["database", "sqlcipher", "path", "data", "upgrade notes"],
    view: view(() => import("./pages/storage").then((m) => ({ default: m.StorageSettings }))),
  },
  {
    id: "modules",
    title: "settings:pages.modules.title",
    description: "settings:pages.modules.description",
    category: "extensions",
    icon: Puzzle,
    keywords: ["plugins", "extensions", "features", "enable", "disable"],
    view: view(() => import("./pages/modules").then((m) => ({ default: m.ModulesSettings }))),
  },
  {
    id: "permissions",
    title: "settings:pages.permissions.title",
    description: "settings:pages.permissions.description",
    category: "extensions",
    icon: KeyRound,
    keywords: ["capabilities", "access", "privacy"],
    view: view(() =>
      import("./pages/permissions").then((m) => ({ default: m.PermissionsSettings })),
    ),
  },
  {
    id: "diagnostics",
    title: "settings:pages.diagnostics.title",
    description: "settings:pages.diagnostics.description",
    category: "advanced",
    icon: ScrollText,
    keywords: ["log", "debug", "troubleshoot", "support", "ipc", "paths"],
    view: view(() =>
      import("./pages/diagnostics").then((m) => ({ default: m.DiagnosticsSettings })),
    ),
  },
  {
    id: "about",
    title: "settings:pages.about.title",
    description: "settings:pages.about.description",
    category: "advanced",
    icon: Info,
    keywords: ["version", "license", "open source"],
    view: view(() => import("./pages/about").then((m) => ({ default: m.AboutSettings }))),
  },
]

let showcasePage: PageDefinition | undefined

/**
 * Development tools, present only in `vite dev` (`tauri dev`). Vite
 * replaces `import.meta.env.DEV` with `false` in builds, so release builds
 * contain neither the page nor its code.
 */
function developmentPages(): PageDefinition[] {
  if (!import.meta.env.DEV) return []
  showcasePage ??= {
    id: "dev-dialogs",
    title: "settings:pages.devDialogs.title",
    description: "settings:pages.devDialogs.description",
    category: "advanced",
    icon: AppWindow,
    keywords: ["dialog", "modal", "showcase", "design", "development"],
    view: view(() => import("@/dev/dialog-showcase").then((m) => ({ default: m.DialogShowcase }))),
  }
  return [showcasePage]
}

let pluginsPage: PageDefinition | undefined

/**
 * Experimental pages, present only in builds that opt in. The plugin
 * marketplace needs `VITE_DECKPILOT_PLUGINS=1` at build time; Vite replaces
 * the check with a constant, so normal and release builds contain neither
 * the page nor its code.
 */
function experimentalPages(): PageDefinition[] {
  if (import.meta.env.VITE_DECKPILOT_PLUGINS !== "1") return []
  pluginsPage ??= {
    id: "plugins",
    title: "settings:pages.plugins.title",
    description: "settings:pages.plugins.description",
    category: "extensions",
    icon: Store,
    keywords: ["marketplace", "plugins", "install", "community", "registry", "experimental"],
    view: view(() =>
      import("@/plugins/marketplace").then((m) => ({ default: m.InstalledPluginsSettings })),
    ),
  }
  return [pluginsPage]
}

// `{{ … }}` (the reference syntax) is a value, so it is never read as a
// placeholder of the translation.
const resolve = (page: PageDefinition): SettingsPageEntry => ({
  ...page,
  title: i18n.t(page.title),
  description: i18n.t(page.description, { syntax: "{{ … }}" }),
})

/**
 * Every settings page, with the pages of enabled modules under Extensions,
 * in the window's language (a module's own name is its manifest's).
 */
export function settingsPages(modules: ModuleInfo[]): SettingsPageEntry[] {
  const fromModules = moduleSettingsPages(modules).map((page): SettingsPageEntry => ({
    id: page.id,
    title: page.module.name,
    description: i18n.t("settings:pages.module.description", { module: page.module.name }),
    category: "extensions",
    icon: page.icon,
    keywords: ["module", page.module.id, ...page.module.config.map((spec) => spec.label)],
    view: page.view ?? null,
    module: page.module,
  }))
  return [
    ...CORE_PAGES.map(resolve),
    ...experimentalPages().map(resolve),
    ...fromModules,
    ...developmentPages().map(resolve),
  ]
}

export function findSettingsPage(modules: ModuleInfo[], id: string) {
  return settingsPages(modules).find((page) => page.id === id)
}

/** Pages grouped by category, in display order; empty categories are left out. */
export function groupedSettings(pages: SettingsPageEntry[]) {
  return settingsCategories()
    .map((category) => ({
      ...category,
      pages: pages.filter((page) => page.category === category.id),
    }))
    .filter((group) => group.pages.length > 0)
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
