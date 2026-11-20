/// <reference types="node" />
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"

import { afterEach, describe, expect, it } from "vitest"

import { checkI18n, parseJsonStrict, placeholders } from "../../scripts/i18n-check.mjs"

let root = ""

afterEach(() => {
  if (root) rmSync(root, { recursive: true, force: true })
  root = ""
})

/** A fixture project: locale files (raw text) and source files. */
function fixture(locales: Record<string, string>, sources: Record<string, string>) {
  root = mkdtempSync(path.join(tmpdir(), "i18n-check-"))
  for (const [file, text] of Object.entries(locales)) {
    mkdirSync(path.dirname(path.join(root, "locales", file)), { recursive: true })
    writeFileSync(path.join(root, "locales", file), text)
  }
  mkdirSync(path.join(root, "src"), { recursive: true })
  for (const [file, text] of Object.entries(sources)) {
    writeFileSync(path.join(root, "src", file), text)
  }
  return (allowlist: string[] = []) =>
    checkI18n({
      localesDir: path.join(root, "locales"),
      namespaces: ["common"],
      languages: ["en", "pl"],
      sourceDirs: [path.join(root, "src")],
      allowlist,
    })
}

const json = (value: unknown) => JSON.stringify(value, null, 2)

const without = (object: Record<string, string>, key: string) =>
  Object.fromEntries(Object.entries(object).filter(([k]) => k !== key))

const validEn = {
  save: "Save",
  hello: "Hello, {{name}}",
  link: "Open <b>{{file}}</b>",
  items_one: "{{count}} item",
  items_other: "{{count}} items",
}
const validPl = {
  save: "Zapisz",
  hello: "Cześć, {{name}}",
  link: "Otwórz <b>{{file}}</b>",
  items_one: "{{count}} element",
  items_few: "{{count}} elementy",
  items_many: "{{count}} elementów",
  items_other: "{{count}} elementu",
}
const validSource = `
  t("common:save"); t("common:hello", { name }); t("common:items", { count })
  <Trans i18nKey="common:link" />
`

describe("i18n check", () => {
  it("passes consistent translations", () => {
    const run = fixture(
      { "en/common.json": json(validEn), "pl/common.json": json(validPl) },
      { "app.tsx": validSource },
    )
    expect(run()).toEqual([])
  })

  it("finds missing and extra keys between languages", () => {
    const withoutSave = without(validPl, "save")
    const run = fixture(
      {
        "en/common.json": json(validEn),
        "pl/common.json": json({ ...withoutSave, extra: "Nadmiar" }),
      },
      { "app.tsx": validSource },
    )
    expect(run()).toEqual([
      'pl: missing key "common:save" (in en)',
      'pl: extra key "common:extra" (not in en)',
    ])
  })

  it("finds placeholders and markup that differ", () => {
    const run = fixture(
      {
        "en/common.json": json(validEn),
        "pl/common.json": json({ ...validPl, hello: "Cześć, {{user}}", link: "Otwórz {{file}}" }),
      },
      { "app.tsx": validSource },
    )
    expect(run()).toEqual([
      'pl: "common:hello" placeholders differ from en; missing {{name}}; unexpected {{user}}',
      'pl: "common:link" placeholders differ from en; missing <b>',
    ])
  })

  it("requires each language's plural forms", () => {
    const withoutMany = without(validPl, "items_many")
    const run = fixture(
      {
        "en/common.json": json({ ...validEn, items_few: "{{count}} items" }),
        "pl/common.json": json(withoutMany),
      },
      { "app.tsx": validSource },
    )
    expect(run()).toEqual([
      'en: "common:items" has plural forms few, one, other; en needs exactly one, other',
      'pl: "common:items" has plural forms few, one, other; pl needs exactly one, few, many, other',
    ])
  })

  it("reports duplicate keys and malformed JSON", () => {
    const run = fixture(
      {
        "en/common.json": `{ "save": "Save", "save": "Store" }`,
        "pl/common.json": `{ "save": "Zapisz", }`,
      },
      { "app.tsx": `t("common:save")` },
    )
    const problems = run()
    expect(problems).toContain('en/common.json: duplicate key "save"')
    expect(problems.some((p) => p.startsWith("pl/common.json: malformed JSON"))).toBe(true)
  })

  it("reports unknown namespaces, unknown keys and unused keys", () => {
    const run = fixture(
      {
        "en/common.json": json(validEn),
        "pl/common.json": json(validPl),
        "en/other.json": "{}",
      },
      { "app.tsx": `t("common:save"); t("nope:thing"); t("common:missing")` },
    )
    expect(run()).toEqual([
      'en/other.json: unknown namespace "other"',
      'src/app.tsx: unknown namespace "nope" in "nope:thing"'.replace(
        "src/app.tsx",
        path.relative(process.cwd(), path.join(root, "src/app.tsx")),
      ),
      `${path.relative(process.cwd(), path.join(root, "src/app.tsx"))}: unknown key "common:missing"`,
      'en: unused key "common:hello" (no source mentions it; see the allowlist)',
      'en: unused key "common:link" (no source mentions it; see the allowlist)',
      'en: unused key "common:items" (no source mentions it; see the allowlist)',
    ])
  })

  it("accepts keys of the dynamic allowlist and literal maps of keys", () => {
    const run = fixture(
      { "en/common.json": json(validEn), "pl/common.json": json(validPl) },
      {
        "app.tsx": `const KEYS = { a: "common:save", b: "common:hello" } as const`,
      },
    )
    expect(run(["common:link", "common:it*"])).toEqual([])
  })
})

describe("strict JSON reader", () => {
  it("parses like JSON.parse and lists duplicates with their path", () => {
    const { value, duplicates } = parseJsonStrict(
      `{"a": {"b": 1, "b": 2}, "c": [true, null, "x\\n"]}`,
    )
    expect(value).toEqual({ a: { b: 2 }, c: [true, null, "x\n"] })
    expect(duplicates).toEqual(["a.b"])
  })

  it("rejects malformed JSON", () => {
    expect(() => parseJsonStrict(`{"a": 1,}`)).toThrow()
    expect(() => parseJsonStrict(`{"a": 1} x`)).toThrow()
    expect(() => parseJsonStrict(`{"a": "unterminated}`)).toThrow()
  })

  it("finds placeholders and markup", () => {
    expect([...placeholders("Hi {{name}}, <b>{{count, number}}</b>")]).toEqual([
      "{{name}}",
      "{{count}}",
      "<b>",
    ])
  })
})
