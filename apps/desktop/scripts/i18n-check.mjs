#!/usr/bin/env node
/**
 * Consistency check of DeckPilot's translations (docs/i18n.md, "Tooling").
 *
 *   pnpm --filter @deckpilot/desktop i18n:check
 *
 * Reads `src/i18n/locales/<language>/<namespace>.json` and the sources
 * under `src/`, and fails (exit code 1, every problem listed) on:
 * - malformed JSON, a duplicate key in an object, a value that is neither
 *   a string nor an object of strings;
 * - a namespace file that is not in `NAMESPACES` (src/i18n/config.ts), or
 *   a namespace of `NAMESPACES` without a file, for any language;
 * - a key that exists in one language but not in another;
 * - placeholders (`{{name}}`) or markup tags (`<name>`, see `Trans`) that
 *   differ between the languages of a key;
 * - plural forms that are not exactly the language's set (English: one,
 *   other; Polish: one, few, many, other);
 * - a key the sources use (`t("ns:key")`, `i18nKey="ns:key"`) that does
 *   not exist, or a namespace the sources use that does not exist;
 * - a key no source mentions, unless it matches a pattern of the dynamic
 *   keys allowlist (`scripts/i18n-dynamic-keys.json`).
 * Exit code 0 when all pass.
 */
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs"
import path from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

/** The plural categories each language must provide (Intl.PluralRules). */
export const PLURAL_FORMS = {
  en: ["one", "other"],
  pl: ["one", "few", "many", "other"],
}

const ALL_PLURAL_SUFFIXES = ["zero", "one", "two", "few", "many", "other"]
const PLURAL_RE = new RegExp(`^(.*)_(${ALL_PLURAL_SUFFIXES.join("|")})$`)

// ---------------------------------------------------------------------------
// A JSON reader that reports duplicate keys (JSON.parse keeps the last one).

class JsonError extends Error {}

/**
 * Parses `text` like JSON.parse; also returns the dotted paths of keys
 * that occur twice in the same object.
 */
export function parseJsonStrict(text) {
  let i = 0
  const duplicates = []
  const fail = (message) => {
    const line = text.slice(0, i).split("\n").length
    throw new JsonError(`${message} at line ${line}`)
  }
  const space = () => {
    while (i < text.length && " \t\n\r".includes(text[i])) i++
  }
  const string = () => {
    if (text[i] !== '"') fail("expected a string")
    const start = i
    i++
    while (i < text.length && text[i] !== '"') {
      if (text[i] === "\\") i++
      else if (text[i] === "\n") fail("unterminated string")
      i++
    }
    if (text[i] !== '"') fail("unterminated string")
    i++
    try {
      return JSON.parse(text.slice(start, i))
    } catch {
      return fail("invalid string escape")
    }
  }
  const value = (where) => {
    space()
    const c = text[i]
    if (c === "{") return object(where)
    if (c === "[") return array(where)
    if (c === '"') return string()
    const literal = /^(?:true|false|null|-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?)/.exec(
      text.slice(i),
    )
    if (!literal) fail("unexpected token")
    i += literal[0].length
    return JSON.parse(literal[0])
  }
  const array = (where) => {
    i++
    const out = []
    space()
    if (text[i] === "]") {
      i++
      return out
    }
    for (;;) {
      out.push(value(`${where}[${out.length}]`))
      space()
      if (text[i] === ",") {
        i++
        continue
      }
      if (text[i] === "]") {
        i++
        return out
      }
      fail("expected , or ]")
    }
  }
  const object = (where) => {
    i++
    const out = {}
    space()
    if (text[i] === "}") {
      i++
      return out
    }
    for (;;) {
      space()
      const key = string()
      const keyPath = where ? `${where}.${key}` : key
      if (Object.hasOwn(out, key)) duplicates.push(keyPath)
      space()
      if (text[i] !== ":") fail("expected :")
      i++
      out[key] = value(keyPath)
      space()
      if (text[i] === ",") {
        i++
        continue
      }
      if (text[i] === "}") {
        i++
        return out
      }
      fail("expected , or }")
    }
  }
  const result = value("")
  space()
  if (i < text.length) fail("unexpected content after the JSON value")
  return { value: result, duplicates }
}

// ---------------------------------------------------------------------------
// Resources

/** Leaf keys of a resource object, dotted; non-string leaves are problems. */
function flatten(object, prefix, problems, where) {
  const out = new Map()
  for (const [key, value] of Object.entries(object)) {
    const full = prefix ? `${prefix}.${key}` : key
    if (typeof value === "string") out.set(full, value)
    else if (value && typeof value === "object" && !Array.isArray(value)) {
      for (const [k, v] of flatten(value, full, problems, where)) out.set(k, v)
    } else problems.push(`${where}: "${full}" must be a string or an object of strings`)
  }
  return out
}

/** `{{name}}` / `{{name, format}}` placeholders and `<tag>` markup of a text. */
export function placeholders(text) {
  const names = new Set()
  for (const match of text.matchAll(/\{\{\s*([^},\s]+)[^}]*\}\}/g)) names.add(`{{${match[1]}}}`)
  for (const match of text.matchAll(/<\/?([A-Za-z0-9_]+)\s*\/?>/g)) names.add(`<${match[1]}>`)
  return names
}

const sameSet = (a, b) => a.size === b.size && [...a].every((x) => b.has(x))

/** `NAMESPACES` of src/i18n/config.ts. */
export function readNamespaces(configFile) {
  const text = readFileSync(configFile, "utf8")
  const match = /export const NAMESPACES = \[([\s\S]*?)\]/.exec(text)
  if (!match) throw new Error(`${configFile}: NAMESPACES not found`)
  return [...match[1].matchAll(/"([^"]+)"/g)].map((m) => m[1])
}

function sourceFiles(dir, skip) {
  const out = []
  for (const name of readdirSync(dir)) {
    const file = path.join(dir, name)
    if (skip.some((s) => file.startsWith(s))) continue
    if (statSync(file).isDirectory()) out.push(...sourceFiles(file, skip))
    else if (/\.(?:ts|tsx|js|jsx|mjs)$/.test(name)) out.push(file)
  }
  return out
}

/** An allowlist pattern: `*` matches any characters. */
function patternToRegExp(pattern) {
  const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replaceAll("*", ".*")
  return new RegExp(`^${escaped}$`)
}

/**
 * Every problem of the translations, as readable lines; empty when they
 * are consistent.
 *
 * @param {{
 *   localesDir: string,
 *   namespaces: string[],
 *   languages: string[],
 *   sourceDirs: string[],
 *   allowlist?: string[],
 *   exclude?: string[],
 * }} options `exclude`: source files or folders not to scan.
 */
export function checkI18n({
  localesDir,
  namespaces,
  languages,
  sourceDirs,
  allowlist = [],
  exclude = [],
}) {
  const problems = []
  const [source] = languages
  /** language → namespace → key → text */
  const data = new Map()

  for (const language of languages) {
    const dir = path.join(localesDir, language)
    const perNamespace = new Map()
    data.set(language, perNamespace)
    const files = existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith(".json")) : []
    for (const file of files) {
      const ns = file.slice(0, -".json".length)
      if (!namespaces.includes(ns)) problems.push(`${language}/${file}: unknown namespace "${ns}"`)
    }
    for (const ns of namespaces) {
      const file = path.join(dir, `${ns}.json`)
      const where = `${language}/${ns}.json`
      if (!existsSync(file)) {
        problems.push(`${where}: missing file`)
        continue
      }
      let parsed
      try {
        parsed = parseJsonStrict(readFileSync(file, "utf8"))
      } catch (error) {
        problems.push(`${where}: malformed JSON: ${error.message}`)
        continue
      }
      for (const key of parsed.duplicates) problems.push(`${where}: duplicate key "${key}"`)
      if (!parsed.value || typeof parsed.value !== "object" || Array.isArray(parsed.value)) {
        problems.push(`${where}: must contain an object`)
        continue
      }
      perNamespace.set(ns, flatten(parsed.value, "", problems, where))
    }
  }

  // Keys grouped by base key: plural forms under their base.
  /** language → "ns:base" → { forms: Set<string> | null, texts: string[] } */
  const groups = new Map()
  for (const [language, perNamespace] of data) {
    const byBase = new Map()
    groups.set(language, byBase)
    for (const [ns, keys] of perNamespace) {
      for (const [key, text] of keys) {
        const plural = PLURAL_RE.exec(key)
        const base = `${ns}:${plural ? plural[1] : key}`
        const group = byBase.get(base) ?? { forms: null, plain: false, texts: [] }
        if (plural) (group.forms ??= new Set()).add(plural[2])
        else group.plain = true
        group.texts.push(text)
        byBase.set(base, group)
      }
    }
  }

  for (const [language, byBase] of groups) {
    const expected = PLURAL_FORMS[language]
    for (const [base, group] of byBase) {
      if (group.forms && group.plain) {
        problems.push(`${language}: "${base}" is both a plain key and a plural`)
      }
      if (!group.forms) continue
      if (!expected) {
        problems.push(`${language}: no plural forms are defined for this language`)
        continue
      }
      const want = new Set(expected)
      if (!sameSet(group.forms, want)) {
        problems.push(
          `${language}: "${base}" has plural forms ${[...group.forms].sort().join(", ")}; ` +
            `${language} needs exactly ${expected.join(", ")}`,
        )
      }
    }
  }

  // The same keys and placeholders in every language.
  const sourceGroups = groups.get(source) ?? new Map()
  for (const language of languages.slice(1)) {
    const other = groups.get(language) ?? new Map()
    for (const base of sourceGroups.keys()) {
      if (!other.has(base)) problems.push(`${language}: missing key "${base}" (in ${source})`)
    }
    for (const base of other.keys()) {
      if (!sourceGroups.has(base))
        problems.push(`${language}: extra key "${base}" (not in ${source})`)
    }
    for (const [base, group] of sourceGroups) {
      const translated = other.get(base)
      if (!translated) continue
      const want = new Set(group.texts.flatMap((t) => [...placeholders(t)]))
      for (const text of translated.texts) {
        const have = placeholders(text)
        // Plural forms may leave out {{count}} (e.g. "one hour").
        const missing = [...want].filter((p) => !have.has(p) && !(group.forms && p === "{{count}}"))
        const extra = [...have].filter((p) => !want.has(p))
        if (missing.length > 0 || extra.length > 0) {
          problems.push(
            `${language}: "${base}" placeholders differ from ${source}` +
              (missing.length ? `; missing ${missing.join(", ")}` : "") +
              (extra.length ? `; unexpected ${extra.join(", ")}` : ""),
          )
        }
      }
    }
  }

  // The sources' use of keys.
  const skip = [path.resolve(localesDir), ...exclude.map((p) => path.resolve(p))]
  const files = sourceDirs.flatMap((dir) => sourceFiles(path.resolve(dir), skip))
  const mentioned = new Set()
  const literal = /["'`]([A-Za-z][\w-]*):([A-Za-z0-9_.-]+)["'`]/g
  const call = /(?:\bt|\.t|i18nKey=|i18nKey:)\s*\(?\s*["'`]([A-Za-z][\w-]*):([A-Za-z0-9_.-]+)["'`]/g
  for (const file of files) {
    const text = readFileSync(file, "utf8")
    const where = path.relative(process.cwd(), file)
    for (const match of text.matchAll(literal)) {
      if (namespaces.includes(match[1])) mentioned.add(`${match[1]}:${match[2]}`)
    }
    for (const match of text.matchAll(call)) {
      const [, ns, key] = match
      if (!namespaces.includes(ns)) {
        problems.push(`${where}: unknown namespace "${ns}" in "${ns}:${key}"`)
      } else if (!sourceGroups.has(`${ns}:${key}`)) {
        problems.push(`${where}: unknown key "${ns}:${key}"`)
      }
    }
  }
  const allowed = allowlist.map(patternToRegExp)
  for (const base of sourceGroups.keys()) {
    if (mentioned.has(base) || allowed.some((re) => re.test(base))) continue
    problems.push(`${source}: unused key "${base}" (no source mentions it; see the allowlist)`)
  }
  return problems
}

// ---------------------------------------------------------------------------

function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
  const allowlistFile = path.join(root, "scripts/i18n-dynamic-keys.json")
  const allowlist = existsSync(allowlistFile)
    ? JSON.parse(readFileSync(allowlistFile, "utf8")).keys
    : []
  const problems = checkI18n({
    localesDir: path.join(root, "src/i18n/locales"),
    namespaces: readNamespaces(path.join(root, "src/i18n/config.ts")),
    languages: Object.keys(PLURAL_FORMS),
    sourceDirs: [path.join(root, "src")],
    allowlist,
    // The check's own test: its fixtures use keys that do not exist.
    exclude: [path.join(root, "src/i18n/i18n-check.test.ts")],
  })
  if (problems.length > 0) {
    console.error(`i18n check failed (${problems.length}):`)
    for (const problem of problems) console.error(`  - ${problem}`)
    process.exit(1)
  }
  console.log("i18n check passed")
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main()
}
