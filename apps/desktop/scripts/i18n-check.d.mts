// Types of i18n-check.mjs, for its unit test (src/i18n/i18n-check.test.ts).

export const PLURAL_FORMS: Record<string, string[]>

export function parseJsonStrict(text: string): { value: unknown; duplicates: string[] }

export function placeholders(text: string): Set<string>

export function readNamespaces(configFile: string): string[]

export function checkI18n(options: {
  localesDir: string
  namespaces: string[]
  languages: string[]
  sourceDirs: string[]
  allowlist?: string[]
  exclude?: string[]
}): string[]
