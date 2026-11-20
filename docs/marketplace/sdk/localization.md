# Localization (plugin API 1.6)

A plugin can ship its own translations and show its texts in the language
of the DeckPilot app (English and Polish today, see
[docs/i18n.md](../../i18n.md)): the texts of its manifest (name,
description, action and setting labels, error messages…), the texts of its
dialogs, and whatever it computes at run time (key labels, notifications,
outputs). Everything here needs `"engines": { "pluginApi": "^1.6" }`.

- [Locale files](#locale-files)
- [Manifest texts: `t:` keys](#manifest-texts-t-keys)
- [The fallback chain](#the-fallback-chain)
- [At run time: `ctx.locale()` and `ctx.t(...)`](#at-run-time-ctxlocale-and-ctxt)
- [Dialogs](#dialogs)
- [When the language changes](#when-the-language-changes)
- [Security](#security)
- [Testing](#testing)
- [Limits](#limits)

## Locale files

Translations are files of the package, `locales/<tag>.json`, one per
language, declared in the manifest:

```json
{
  "engines": { "deckpilot": ">=0.10.0", "pluginApi": "^1.6" },
  "defaultLocale": "en",
  "locales": ["en", "pl"],
  "name": "t:plugin.name"
}
```

`locales/en.json`:

```json
{
  "plugin": {
    "name": "Repositories",
    "description": "Shows how many pull requests wait for you."
  },
  "actions": {
    "count": { "name": "Count pull requests", "description": "Counts open pull requests." }
  },
  "settings": { "apiToken": "API token" },
  "prs_one": "{{count}} pull request",
  "prs_other": "{{count}} pull requests"
}
```

`locales/pl.json`:

```json
{
  "plugin": {
    "name": "Repozytoria",
    "description": "Pokazuje, ile pull requestów na Ciebie czeka."
  },
  "actions": {
    "count": { "name": "Policz pull requesty", "description": "Liczy otwarte pull requesty." }
  },
  "settings": { "apiToken": "Token API" },
  "prs_one": "{{count}} pull request",
  "prs_few": "{{count}} pull requesty",
  "prs_many": "{{count}} pull requestów",
  "prs_other": "{{count}} pull requestu"
}
```

- The format is i18next JSON: an object of strings, flat (`"plugin.name"`)
  or nested (`{"plugin": {"name": …}}`), keys made of letters, digits, `_`
  and `-` separated by dots.
- `{{name}}` is a placeholder filled from the parameters of
  [`ctx.t`](#at-run-time-ctxlocale-and-ctxt). Formatting options
  (`{{value, number}}`) and nesting (`$t(…)`) are not supported.
- Plurals use the `Intl.PluralRules` categories as suffixes, chosen by the
  parameter `count`: English needs `_one` and `_other`; Polish `_one`,
  `_few` (2–4, 22–24…), `_many` (0, 5–21, 25…) and `_other` (fractions).
- Translations are **plain text**. A file with an HTML-like tag (`<b>`,
  `</x>`, `<!--`), a control character other than a line break or tab, a
  malformed placeholder, a value that is not a string, or a duplicate key
  is refused when the package is built and when it is installed.
- `defaultLocale` (default `en`) is the language of the manifest's own
  texts, and must be one of `locales`. Tags are canonical: `pl`, `pl-PL`,
  `zh-Hant`, `es-419`.

Build the package as usual (`deckpilot-registry package`, `build.sh`): the
files the manifest names, the locale files included, are packed, hashed and
pinned with the rest ([artifact-contract.md](../artifact-contract.md)).

## Manifest texts: `t:` keys

Any text of the manifest the app shows may name a key of the plugin's own
locale files with the prefix `t:`:

| Field | Example |
| --- | --- |
| `name`, `description`, `publisher.name` | `"name": "t:plugin.name"` |
| `contributes.actions[].name`, `.description`, `.category` | `"name": "t:actions.count.name"` |
| parameter, output, setting, credential and variable `label`/`name` and `description`, `itemLabel`, `nameLabel`, `valueLabel` | `"label": "t:settings.apiToken"` |
| `contributes.errorCodes` messages | `"rate-limited": "t:errors.rateLimited"` |
| `contributes.navigation[].label` | `"label": "t:nav.repositories"` |
| `permissions[].justification`, `dependencies[].justification` | `"justification": "t:permissions.network"` |

Identifiers are never translated: the plugin id, action ids, parameter and
setting keys, choice values, variable ids, error codes, permission kinds
and capability names stay exactly as written. A plain text (without
`t:`) is shown as it is, in every language. A manifest that requires a
plugin API below 1.6 reads `t:…` as plain text, so older manifests are
unchanged.

The manifest validator refuses a `t:` text whose key is malformed, and a
`t:` text in a manifest without `locales`. The field limits (a name ≤ 48
characters…) apply to the translations too: a longer one is cut.

## The fallback chain

A key is looked up, in order, in:

1. the app's language as a tag (`pl-PL`),
2. its language without the region (`pl`),
3. the plugin's `defaultLocale`,
4. `en`,
5. and when none has it: the plain text where there is one, else a label
   made from the key's last segment (`settings.apiToken` → `Api token`).

Resolving never fails and never stops a plugin from loading. A key that no
locale has is written to the agent's log once per plugin and key (debug
level), so it shows up in development without flooding the log.

The app's language is the one its window shows (Settings → General →
Language), so today `en` or `pl`; a plugin should still provide regional
files when they differ (`pl-PL`) and they are used when asked for.

## At run time: `ctx.locale()` and `ctx.t(...)`

```rust
use deckpilot_plugin_sdk::prelude::*;

fn count(ctx: &mut Context<'_>, _: &Args<'_>) -> ActionResult {
    let open = 3; // from the API
    let label = ctx.t("prs", &[("count", open)])?; // "3 pull requesty" in Polish
    let language = ctx.locale()?; // "pl", "pl-PL", "en"…
    Ok(Output::ok().with("label", label).with("language", language))
}
```

| Call | Op | Answer |
| --- | --- | --- |
| `ctx.locale()` | `locale.get` | The app's language as a BCP 47 tag; nothing else about the system |
| `ctx.t(key, &[("name", value)…])` | `locale.translate` | The plugin's own text for `key`, with `{{name}}` filled; `count` also picks the plural form |
| `ctx.text(key)` | `locale.translate` | `ctx.t` without parameters |

Parameter values are texts (at most 256 characters, no control characters)
or numbers (`TranslateArg`); mix them with `TranslateArg::from(..)`:
`ctx.t("greeting", &[("name", TranslateArg::from("Ada")), ("count", 2.into())])`.
At most 16 parameters. A malformed key or parameter is refused by the SDK
before anything is sent (`BadRequest`). Both calls need no permission and
are allowed in every run (also from a schedule, and in a key's state
call); `dp_init` and `dp_shutdown` have no invocation, so there they are
`unavailable`.

An agent older than plugin API 1.6 answers `Unsupported`; a plugin that
should still work there falls back itself:
`ctx.t("prs", &[("count", n)]).unwrap_or_else(|_| format!("{n} PRs"))`.

## Dialogs

A plugin dialog ([dialogs.md](dialogs.md)) may give its title, message,
button labels, field labels and descriptions and option labels and
descriptions as `t:` keys; the agent resolves them in the app's language
before the app draws the dialog (as plain text). Ids, values and defaults
are never translated.

```rust
ctx.dialog(Dialog::confirm("t:dialogs.delete.title").confirm_label("t:dialogs.delete.ok"))?;
```

## When the language changes

There is no export to implement. The app tells the agent its language when
it starts and whenever the user changes it (IPC protocol 13, `setLocale`);
then:

- the plugin's manifest texts are shown in the new language at once (the
  Marketplace, the plugin's settings, the Action Library, the toolbar);
- keys titled with an action's default name (one of its translations, not
  a title the user wrote) are redrawn in the new language;
- the agent asks again for the state of the plugin's keys (`dp_state`), so
  labels the plugin computes with `ctx.t` follow;
- the next invocation reads the new language.

Titles the user wrote are never changed.

## Security

- The locale files are files of the signed package: their digests are in
  the registry's pins (or the bundled pins of a first-party plugin) and in
  the integrity record the agent checks at every load. A changed file makes
  the package or the installed plugin refused; translations are never
  downloaded on their own.
- `locale.translate` carries a key only: no plugin id, no namespace. The
  agent resolves it against the calling plugin's own catalog, so a plugin
  can neither read nor override another plugin's or DeckPilot's texts.
- Every text is plain text in the app and on keys: nothing is rendered as
  HTML or markup.

## Testing

`MockHost::localized(locale, messages)` answers `locale.get` with `locale`
and `locale.translate` from `messages` (flattened keys, plural forms by the
English and Polish rules, a label for a missing key), like the agent:

```rust
use deckpilot_plugin_sdk::testing::MockHost;

let mut host = MockHost::localized("pl", &[("prs_few", "{{count}} PR-y")]);
let mut ctx = host.context();
assert_eq!(ctx.t("prs", &[("count", 3)]).unwrap(), "3 PR-y");
```

## Limits

| What | Limit |
| --- | --- |
| Locale files | ≤ 32 per package, each ≤ 64 KiB |
| Keys per file | ≤ 1,024, ≤ 128 characters, ≤ 8 levels deep |
| A translation | ≤ 1,024 characters (and the limit of the field it fills) |
| Tags | language (2–3 letters), optional script (4), optional region (2 letters or 3 digits) |
| `ctx.t` parameters | ≤ 16; names ≤ 32 characters; texts ≤ 256 characters |
