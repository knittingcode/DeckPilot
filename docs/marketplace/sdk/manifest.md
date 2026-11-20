# The manifest (`manifest.json`, manifest version 1)

`manifest.json` sits at the root of the plugin's source directory and of
its package. It is the only thing the agent reads to learn what a plugin
is, asks for and offers: the agent never runs plugin code to discover
metadata. The module must agree with it (the agent and the SDK check,
[below](#the-module-must-agree)).

Sources of truth, in order:

1. The JSON Schema `crates/deckpilot-plugin-manifest/schema/plugin-manifest.schema.json`
   (draft 2020-12; byte-identical copy in `plugins/schemas/`), bundled as
   `deckpilot_plugin_manifest::SCHEMA_JSON`.
2. The typed model and validation in `crates/deckpilot-plugin-manifest`:
   `Manifest::from_json` (structure, unknown fields refused everywhere,
   at most `MAX_MANIFEST_BYTES` = 256 KiB) and `Manifest::validate` (every
   other schema constraint plus the cross-field rules below, all problems
   reported at once with their JSON paths, `ValidationErrors`).
3. The package reader (`deckpilot_plugin_package::verify_package`) and the
   agent, which repeat these checks at install time and before every load.

Validate while you write it: the examples' unit tests run the schema with
`jsonschema` (`manifest_is_valid_against_the_schema`), and
`deckpilot-registry package` runs the full `Manifest::validate` (see
[building.md](building.md#package)).

## Top-level fields

| Field | Required | Rule |
| --- | --- | --- |
| `$schema` | no | Any string ≤ 256; the examples use `https://knittingcode.github.io/deckpilot-plugins/v1/schemas/plugin-manifest.schema.json` |
| `manifestVersion` | yes | `1` (`MANIFEST_VERSION`) |
| `id` | yes | `<publisher>.<name>` (`PluginId`): each part `[a-z][a-z0-9]*(-[a-z][a-z0-9]*)*`, publisher 2-24, name 2-32 characters, 57 in all. Must start with `publisher.id` and a dot. A hyphen is always followed by a letter, so the lower camel case (`acme.developer-tools` → `plugins.acme.developerTools`) is unique |
| `name` | yes | Display name, 1-48 characters |
| `version` | yes | `X.Y.Z` with an optional `-alpha.N`, `-beta.N` or `-rc.N` (N ≥ 1); no build metadata, no leading zeros, ≤ 32 characters. Must equal `Plugin::new(_, version)` |
| `publisher` | yes | `{ "id", "name", "url"? }`: `id` as above and not reserved (`RESERVED_PUBLISHERS`: `deckpilot`, `knittingcode`, `core`, `system`, `plugin`, `test`, the built-in module ids, …), `name` 1-64, `url` https ≤ 512 |
| `description` | yes | 1-300 characters |
| `homepage` | no | https URL ≤ 512 |
| `repository` | no | `{ "type": "git", "url": "https://github.com/<owner>/<repo>", "directory"? }` (`directory` a relative path) |
| `license` | yes | SPDX expression, 1-64 characters. The package must contain a `LICENSE` file |
| `keywords` | no | ≤ 10 unique, each `[a-z0-9-]{2,24}` |
| `categories` | no | ≤ 3 unique of `developer-tools`, `productivity`, `communication`, `monitoring`, `media`, `smart-home`, `other` |
| `engines` | yes | See [Compatibility](#compatibility-engines-and-platforms) |
| `runtime` | in practice yes | `{ "kind": "wasm", "abi": 1 }`. Optional in the schema only so #191 manifests still parse; a package without it is the old JavaScript contract and is refused (`PackageError::UnsupportedRuntime`, [artifact-contract.md](../artifact-contract.md)) |
| `entry` | yes | Informational relative path of the source entry (`src/lib.rs`); the agent ignores it |
| `platforms` | no | Non-empty unique list of `macos`, `windows`, `linux`; omitted means all |
| `icons` | yes | `{ "128": "assets/…png" }` required; `32` and `256` optional. Paths `assets/…​.png` ≤ 128 characters. The agent shows `icons.128` only if it is a PNG of at most 256 KiB whose sha256 matches the install record (`MAX_ICON_BYTES`) |
| `assets` | no | `keyIcons` (≤ 64 PNGs by `[a-z][a-z0-9-]{0,47}` name), `screenshots` (≤ 8 PNGs), `readme` (`"README.md"`), `changelog` (`"CHANGELOG.md"`). Everything named here is packaged |
| `permissions` | yes | Array, ≤ 16; may be empty. See [permissions.md](permissions.md) |
| `dependencies` | no | Plugin API 1.4 (needs `"engines": { "pluginApi": "^1.4" }`): ≤ 16 plugins (`<publisher>.<name>`) or built-in modules (`projects`) the plugin needs, each `{ "id", "version" (a range as `engines.deckpilot`), "optional"?, "justification"? }`; never the plugin's own id. A dependency grants nothing. See [dependencies.md](../dependencies.md) |
| `locales` | no | Plugin API 1.6 (needs `"engines": { "pluginApi": "^1.6" }`): ≤ 32 unique canonical tags (`en`, `pl`, `pl-PL`), each packaged as `locales/<tag>.json`, a JSON object of plain-text strings. With it, the texts the app shows may be `"t:<key>"` ([localization.md](localization.md)) |
| `defaultLocale` | no | Plugin API 1.6: the language of the manifest's own texts, one of `locales`; `en` when absent. The last locale tried before `en` |
| `supports` | no | `{ "device": { "requires"?, "uses"? } }` with `keys`, `keyDisplays`, `dials`, `touchStrip`, `brightness`. Accepted, not used by the agent in this version |
| `contributes` | yes | `actions`, `variables`, `settings`, `credentials`, `errorCodes`, `navigation`, `views`, `visualAssets`; all optional |

Every text the app shows (`name`, `description`, `publisher.name`, action
names, descriptions and categories, parameter, output, setting, credential
and variable labels and descriptions, `errorCodes` messages, navigation
labels, visual asset names and permission and dependency justifications) may name a key of the
plugin's own translations as `"t:key.path"` in a manifest of plugin API 1.6
with `locales` ([localization.md](localization.md)). Ids and keys are never
translated; below 1.6 a `t:` text is plain text.

## `contributes.actions[]`

At most 64 actions (`MAX_ACTIONS`). Each:

| Field | Required | Rule |
| --- | --- | --- |
| `id` | yes | `[a-z][a-z0-9]*(-[a-z0-9]+)*`, 1-48 characters, unique. Must equal `Action::new(id, _)`. The agent registers it as action type `<plugin id>.<id>` |
| `name` | yes | 1-48 characters |
| `description` | yes | 1-300 characters |
| `category` | yes | 1-32 characters (the group in the editors' pickers) |
| `parameters` | yes | ≤ 32 (`MAX_PARAMS`); see [actions.md](actions.md#parameters) |
| `node` | no | Workflow node contract: `version` (≥ 1), `icon` (`[a-z0-9-]{1,64}`), `outputs` (≤ 32), `timeoutMs` (1,000-600,000, default 30,000: the invocation's wall-clock limit), `retrySafe`, `idempotent`, `effect` (`readOnly`, `opens`, `modifies`; a warning when `modifies` is `retrySafe`). `cancellable` is accepted; the agent treats every plugin action as cancellable. There is no `changesContext` |
| `requiresProject`, `deprecated`, `replacedBy` | no | `replacedBy` names another action of this plugin and needs `deprecated: true` |
| `state` | no | Key state refresh (`onEvents` or `every` with `intervalSeconds` 1-86,400; needs `effect: readOnly`). From plugin API 1.2 the agent asks the action's state handler for keys shown on a device ([actions.md](actions.md#key-state)), never more often than every 5 s; it needs the `device` permission with `state: true`. An agent below 1.2 accepts it and asks nothing |
| `device` | no | `{ "requires": [...] }`. Accepted, not used: it never makes an action bindable to a dial or a touch strip (use `controls`) |
| `requiresDependencies` | no | Plugin API 1.4: ids of declared `dependencies` (usually optional ones) the action needs; while one is missing or off, only this action is unavailable ([dependencies.md](../dependencies.md#1-the-manifest)) |
| `controls` | no | Plugin API 1.3: the controls and inputs the action can be bound to; see [Controls](#controls). Without it: keys only |
| `editorNavigation` | no | Plugin API 1.7: what running the action does to the page or layout a device shows, for the Device Editor's preview; see [Editor navigation](#editor-navigation) |
| `title` | no | Plugin API 1.8: suggestions for the action's keys' titles; see [Titles](#titles) |

### Titles

`title` (plugin API 1.8; the manifest must declare `^1.8`, the validator
refuses it below) says what the action suggests for the titles of its keys.
It is **data only** and needs no permission: the title editor is always
DeckPilot's own, the same for every key, and the user's own title and look
always win ([key-text.md](../../key-text.md#title-precedence)).

```json
"title": { "default": "CPU", "dynamic": true, "position": "bottom", "supports": ["position", "color"] }
```

| Field | Rule |
| --- | --- |
| `default` | 1–32 characters (a `t:` key with `locales`): the title a key shows while neither the key nor the action has a label and the action keeps its type's name |
| `dynamic` | `true`: the action sets its title while it runs (its key state's `label`); a title the user wrote is kept over it |
| `position` | `top`, `middle` or `bottom`: used while the key sets no position |
| `supports` | The parts of the look the editor offers for the action's keys: `visibility`, `position`, `font`, `style`, `color`, `layout`, each at most once; absent: all. The others are shown disabled |

### Editor navigation

`editorNavigation` (plugin API 1.7; the manifest must declare `^1.7`, the
validator refuses it below) tells the Device Editor's interactive preview
what a key running the action does to the view, so that double-clicking
the key in the editor follows it. It is **data only**: the editor moves its
own view with DeckPilot's navigation code, and **the plugin is never
called** for it. Nothing in it is executable, and a plugin can describe
only its own actions.

```json
"editorNavigation": { "operation": "folder.open", "targetParameter": "page" }
```

| Field | Rule |
| --- | --- |
| `operation` | One of `folder.open`, `folder.back`, `page.next`, `page.previous`, `layout.switch`, `layout.back`; nothing else is accepted |
| `targetParameter` | Required for `folder.open` (the parameter holds a page id) and `layout.switch` (a layout id), refused for the others; must name a `text` parameter of this action |
| `pageParameter` | Only for `layout.switch`: a `text` parameter holding the page of that layout to show |

The declaration describes the editor only: the action itself still has to
do what it says when it runs. A key whose target parameter is empty, or
not an id, does not navigate in the editor. See
[editor-navigation-and-simulation.md](../../editor-navigation-and-simulation.md).

### Controls

`controls` (plugin API 1.3, needs `"engines": { "pluginApi": "^1.3" }`)
says, per control kind, which inputs the action can be bound to. The same
shape as `ActionControls` in `deckpilot-core`
([device-controls.md](../../device-controls.md)):

```json
"controls": { "key": ["press", "click", "long-press"],
              "encoder": ["rotate-clockwise", "rotate-counter-clockwise"] }
```

| Kind | Inputs it may list |
| --- | --- |
| `key` | `press`, `release`, `click`, `double-click`, `long-press` |
| `encoder` | `rotate-clockwise`, `rotate-counter-clockwise`, `press`, `release`, `click`, `double-click`, `long-press` (the last five only on encoders that can be pressed) |
| `touchZone` | `tap`, `long-touch`, `swipe` |

- At least one kind; each kind lists 1-8 inputs (`MAX_INPUTS_PER_KIND`),
  each once, and only inputs a control of the kind can produce (no
  rotation on a key, no tap on an encoder). A kind that is absent is not
  supported: the editors do not offer the action there, saving such a
  binding is refused, and the agent does not run one that was stored.
- Without `controls` the action works on **keys only**, with `press`,
  `release`, `click`, `double-click` and `long-press`: the default of every
  manifest before 1.3. `device.requires` (even `["dials"]`) adds no kind;
  declare `controls` for encoders and touch zones.
- An encoder action runs once per rotation event the device reports,
  whatever its number of detents: nothing passes the amount on.
- In the SDK: `Action::controls(Controls::new().key([..]).encoder([..]))`
  ([actions.md](actions.md#controls)); `manifest_mismatches` compares it
  with the manifest.

### Outputs (`node.outputs[]`)

`{ "key", "label", "kind"?, "always"?, "sensitive"? }`: `key` camelCase
`[a-z][A-Za-z0-9]{0,31}`, `label` 1-48, `kind` one of `text` (default),
`integer`, `boolean`, `path`, `url`, `projectKey`. **The agent keeps only
declared outputs**: anything else an action returns in `Output::with` is
dropped (`convert::outcome`, `PluginActionHandler::result`). In workflows
every output value becomes text (`engine.rs` `text`); `invokePluginAction`
returns them as JSON.

## `contributes.variables[]`

The plugin's own variables, published with `Context::publish_variable`
([capabilities.md](capabilities.md#variables)). At most 64.

| Field | Rule |
| --- | --- |
| `id` | Relative path `[a-z][A-Za-z0-9_]{0,31}` with up to two more `.`-separated segments, ≤ 100 characters, unique. Read in workflows as `{{ plugins.<publisherCamel>.<nameCamel>.<id> }}`, e.g. `plugins.acme.developerTools.openPrs` |
| `name`, `description` | 1-48; ≤ 300 |
| `valueType` | `{ "type": "string" \| "integer" \| "boolean" \| "json" }`; a published value of another type is refused |
| `scope` | `machine`, `profile` or `project`. **Only `machine` can be published in this version**; the others are refused with `unavailable` |
| `requires`, `sensitive`, `deprecated` | Optional; `requires` only `["project"]` |

## `contributes.settings`

`{ "portabilityVersion"?, "fields": [...] }`, at most 32 fields. Each field
is a parameter (same `key`, `label`, `description`, `kind`, `required`,
`default` as [action parameters](actions.md#parameters)) plus
`portability` (`portable`, `machineSpecific` (default) or `sensitive`).
A `text` setting named by a `network.userOrigins` slot holds a server the
user enters ([permissions.md](permissions.md#user-entered-origins)); the
agent normalizes it when saved and asks for consent to that server. An
`openUrl` permission may list the same setting in its `userOrigins` to open
pages of that server ([permissions.md](permissions.md#pages-of-a-server-the-user-entered)).
The key `privateHosts` is reserved (`GENERATED_SETTING_KEY`, generated by the agent for `privateNetwork`). The
user edits them with `setPluginSettings` (refused with `invalidData` unless
every value fits its field); the plugin reads them with
`Context::settings` ([capabilities.md](capabilities.md#settings)). There
are no secret settings: secrets are credential slots.

## `contributes.credentials[]`

Credential slots, at most 8. A slot is a named place for one secret the
user stores (`setPluginCredential`); the plugin only ever names it
([capabilities.md](capabilities.md#credentials)).

| Field | Rule |
| --- | --- |
| `key` | camelCase, unique; must be listed in a `credentials` permission's `refs`, and every ref must be a slot |
| `label`, `description` | 1-48; ≤ 300 |
| `hosts` | 1-8 exact hosts (0-8 with `userOrigin`), each covered by the `network` permission's `hosts`. The secret goes to these origins (https) and nowhere else |
| `userOrigin` | The `setting` of a `network.userOrigins` slot: the secret also goes to the server the user entered there, once they allowed it ([permissions.md](permissions.md#user-entered-origins)) |
| `schemes` | Non-empty subset of `bearer`, `basic`, `header`, `query` |
| `headerName` | Required with `header`: `[A-Za-z0-9-]{1,64}` |
| `queryName` | Required with `query`: `[A-Za-z0-9_.-]{1,64}` |
| `setupUrl` | https URL where the user creates the secret |

## `contributes.errorCodes`

`{ "<code>": "<meaning, 1-200 characters>" }`, at most 64. Codes are
`[a-z][a-z0-9]*(-[a-z0-9]+)*`, ≤ 64 characters, and must not redeclare a
shared code (`SHARED_ERROR_CODES`). See
[errors-and-versions.md](errors-and-versions.md#error-codes).

## `contributes.navigation` and `contributes.views`

Plugin API 1.3 (needs `"engines": { "pluginApi": "^1.3" }`): up to four
entries in the app's toolbar, each opening a view the app renders itself.
The plugin ships no HTML, URL or code for them.

```json
"navigation": [{ "id": "dashboard", "label": "GitHub",
                 "icon": { "light": "assets/nav.svg", "dark": "assets/nav-dark.svg" },
                 "view": "dashboard" }],
"views": [{ "id": "dashboard", "kind": "variables" }]
```

| Field | Rule |
| --- | --- |
| `navigation` | ≤ 4 entries (`MAX_NAVIGATION_ENTRIES`), ids unique |
| `navigation[].id` | `[a-z][a-z0-9]*(-[a-z0-9]+)*`, 1-32 characters |
| `navigation[].label` | 1-24 characters, no leading or trailing spaces, no control characters |
| `navigation[].icon` | One path for both themes, or `{ "light": ..., "dark": ... }` with at least one of them (a theme without its own file shows the other's). See the icon rules below |
| `navigation[].view` | The `id` of one of `views` |
| `views` | ≤ 8 views, ids unique |
| `views[].id` | As `navigation[].id` |
| `views[].kind` | `variables` (the plugin's variables, live), `actions` (the Action Library filtered to the plugin's actions) or `settings` (the plugin's configuration page) |
| `views[].context` | Optional, plugin API 1.4 (needs `"engines": { "pluginApi": "^1.4" }`): `none` (the default), `device`, `profile` or `device+profile`, the app's toolbar selectors shown with the view. Only the app's chrome follows it: it gives the plugin no device or profile data and no permission |

**Icons.** A path relative to the package root under `assets/`, ending in
`.png`, `.webp` or `.svg` (lowercase): no absolute path, no scheme or
URL, no `..`, no backslash. The file is part of the package (and so of its
integrity record) and is checked when the package is built and when it is
installed:

- PNG and WebP: at most 256 KiB and 512×512 pixels (read from the file's
  header), and the content must be the format the extension names.
- SVG: at most 64 KiB, UTF-8, well-formed, an `<svg>` document. Refused:
  `<script>`, `<foreignObject>`, event handler attributes (`on…`), `href`
  or `xlink:href` other than to an id in the same file (`#id`: no external
  file, `data:` or `javascript:`, also on `<use>`), `<style>` with `url(`
  or `@import`, `url(…)` other than `url(#id)` in attributes, CSS escapes,
  DOCTYPEs, entity declarations and references other than the five XML
  ones, processing instructions (such as `xml-stylesheet`). Elements and
  attributes outside the allow-list (editor metadata, `<image>`, `<a>`,
  animations) are left out.

The agent checks the file against the integrity record and sanitizes an
SVG again every time it serves one; the app renders icons only through
`<img>` from a `data:` URL, and shows its own fallback icon when there is
none to show. Entries are shown only for installed and enabled plugins and
disappear when the plugin is uninstalled.

## `contributes.visualAssets`

Plugin API 1.8 (needs `"engines": { "pluginApi": "^1.8" }`): icons and
backgrounds for DeckPilot's shared Icon and Background Libraries. Data
only: no plugin code runs to list or draw them, and the plugin cannot
replace or extend the pickers.

```json
"visualAssets": {
  "icons": [
    { "id": "sunny", "name": "t:assets.sunny", "asset": "assets/icons/sunny.png", "keywords": ["sun"] }
  ],
  "backgrounds": [
    { "id": "sky", "name": "Sky", "type": "image", "asset": "assets/backgrounds/sky.jpg" },
    { "id": "dusk", "name": "Dusk", "type": "gradient", "colors": ["#ff8800", "#220044"], "angle": 135 }
  ]
}
```

| Field | Rule |
| --- | --- |
| `icons` | ≤ 128 (`MAX_VISUAL_ICONS`) |
| `backgrounds` | ≤ 32 (`MAX_VISUAL_BACKGROUNDS`) |
| `…[].id` | `[a-z][a-z0-9]*(-[a-z0-9]+)*`, 1-48 characters, unique across icons and backgrounds. DeckPilot names the asset `<plugin id>.<id>` (`acme.weather.sunny`), which is what a key stores; it never collides with another plugin's or DeckPilot's own (`deckpilot.<name>`) |
| `…[].name` | 1-48 characters, may be `t:<key>` ([localization.md](localization.md)); shown in the app's language with the usual fallback |
| `icons[].asset` | A PNG under `assets/`: at most 256 KiB and 512×512 pixels |
| `icons[].keywords` | Optional, ≤ 16 search words of 1-32 characters (not translated) |
| `backgrounds[].type` | `image` or `gradient` |
| `backgrounds[].asset` | `image` only, required: a PNG or JPEG (`.png`, `.jpg`, `.jpeg`) under `assets/`: at most 1 MiB and 1024×1024 pixels |
| `backgrounds[].colors` | `gradient` only, required: 2-4 `#rrggbb` colours, evenly spaced |
| `backgrounds[].angle` | `gradient` only: 0-359 degrees as in CSS (0 to the top, 90 to the right); default 180 |

Paths follow the package path rules (no absolute path, scheme, `..` or
backslash). The files are named by the manifest, so they are in the
package's integrity record and the registry's pins like every other file;
the package is refused when it is built or installed if a file is missing,
of another format than its extension (checked by content), too large, or
larger than its pixel limit (read from the header, nothing decoded). The
agent reads a file only from the active version's install directory,
compares its sha256 with the integrity record every time, and decodes it
with limits; a changed file is left out (and the plugin's integrity check
marks it broken as for any other file).

Keys draw a plugin's assets only while the plugin is enabled. Installing a
plugin never changes a key; a key keeps its reference through disable,
update, uninstall and reinstall, draws the device's default background and
no icon of its own while the asset is not there, and draws it again once
the plugin is back. The app reads the list from `PluginSummary.visualAssets`
and the files with `getPluginIcon` and `asset` (runtime feature
`visualAssets`). Images are drawn covering the key (cropped, aspect ratio
kept) and gradients rasterised by DeckPilot's renderer, never by CSS;
nothing is animated.

## Compatibility: `engines` and `platforms`

| Field | Rule | Checked |
| --- | --- | --- |
| `engines.deckpilot` | One comparator or a lower and an upper bound separated by one space: `>=`, `>`, `<=`, `<`, `=` (or none, meaning `=`) followed by `MAJOR.MINOR.PATCH` without leading zeros or pre-release, e.g. `">=0.5.0 <1.0.0"`; ≤ 64 characters; must be satisfiable (`VersionRange`). Matching uses semver precedence **including DeckPilot pre-releases**: `0.6.0-alpha.3` satisfies `>=0.5.0 <1.0.0`, and `0.8.0-beta.1` does not satisfy `>=0.8.0` | By the registry client before install (`incompatible`) and by the agent (`HostEnvironment.app_version` = the agent's version) |
| `engines.pluginApi` | `^MAJOR.MINOR`: same major, at least this minor (`ApiRequirement`). The agent provides **1.8** (`PLUGIN_API` in `apps/agent/src/plugins/service.rs`), so `^1.0` to `^1.8` install; declare `^1.1` only when the plugin cannot work without the time or random bytes, and `^1.2` only when it is useless without key state ([errors-and-versions.md](errors-and-versions.md#versions)). A manifest with `controls`, `navigation` or `views` must declare `^1.3` (the validator refuses a lower level); agents up to 1.2 refuse such a manifest anyway, as they do not know the fields. DeckPilot 0.8.0 is the first release whose agent provides 1.3 (0.6.0 and 0.7.0 provide 1.2), so a plugin declaring `^1.3` should also declare `engines.deckpilot` `">=0.8.0"`. A view's `context`, `dependencies` and `requiresDependencies` need `^1.4`; the first release whose agent provides 1.4 is the one after 0.8.1. The `dialogs` permission needs `^1.5` (the validator refuses it below; [dialogs.md](dialogs.md)); the agent of the release after 0.9.0 is the first to provide 1.5. `locales` and `defaultLocale` need `^1.6` ([localization.md](localization.md)), also the first provided by the agent of the release after 0.9.0. An action's `editorNavigation` needs `^1.7` ([Editor navigation](#editor-navigation)), likewise first provided by the agent of the release after 0.9.0. An action's `title` needs `^1.8` ([Titles](#titles)), likewise first provided by the agent of the release after 0.9.0 | Same |
| `engines.sdk` | Informational semver of the SDK used (`deckpilot_plugin_sdk::SDK_VERSION`, 0.1.0) | `testing::manifest_mismatches` only |
| `platforms` | Omitted means macOS, Windows and Linux | Same; a mismatch is `Incompatibility::Platform` |

## The module must agree

The manifest and the module are written separately, and three checks keep
them together:

- **Action ids.** When the host loads the module it reports the SDK's
  descriptor (`dp_describe`); the supervisor refuses a module whose action
  ids differ from the manifest's (`HostConfig::expect_actions`). The
  invocation fails with `hostFailed`.
- **Parameters.** The agent validates every invocation against the
  manifest's parameters before the host runs (`invalid-parameters`); the
  SDK validates again against `Param` declarations. A parameter the
  manifest declares but the code does not is refused by the SDK as
  unknown.
- **Unit test.** `deckpilot_plugin_sdk::testing::manifest_mismatches(&plugin(), &manifest)`
  lists every difference in id, version, `engines.pluginApi`,
  `engines.sdk`, action ids and order, action `controls`, and per
  parameter `required`, kind,
  bounds, options and default. Both examples assert it is empty.

## Example

`plugins/examples/developer-tools/manifest.json` uses every section:
three permissions, seven actions with outputs, two variables, three
settings, one credential slot and seven error codes.
