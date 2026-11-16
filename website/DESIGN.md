# Design

DeckPilot's websites share one design system, so the product site and the
Plugin Marketplace read as one product. Its home is the Marketplace
repository:

- source: [`knittingcode/deckpilot-plugins`, `website/src/styles/`](https://github.com/knittingcode/deckpilot-plugins/tree/main/website/src/styles)
- documentation (tokens, components, accessibility rules, voice):
  [`website/DESIGN.md` there](https://github.com/knittingcode/deckpilot-plugins/blob/main/website/DESIGN.md)

That document is the reference. This file only says how the system is
shared and what this site adds.

## How it is shared

Three plain CSS files, with no build dependency, are **vendored unchanged**
into [`src/styles/shared/`](src/styles/shared):

| File | Contents |
| --- | --- |
| `tokens.css` | Every token: colour (light and dark), type, spacing, layout, shape, motion |
| `base.css` | Element defaults: body, headings, links, code, focus, reduced motion |
| `components.css` | Component classes (`.btn`, `.card`, `.badge`, `.notice`, `.prose`, `.faq`, ...) |

[`src/styles/shared/SOURCE.json`](src/styles/shared/SOURCE.json) records the
repository, the commit they were taken from and the SHA-256 of each file.

**Never edit the files in `src/styles/shared/`.** A change to a token or a
shared component is made in the Marketplace repository and then taken here.

There is no build-time link between the repositories: each site builds from
files in its own repository, so neither deployment can break the other.
What keeps the copies honest is a check
([`tools/check-design-system.mjs`](tools/check-design-system.mjs)):

| Command | When | Result |
| --- | --- | --- |
| `npm run check:design` | Every CI run | Fails if a shared file differs from its recorded checksum (it was edited here), or a brand file differs from `.github/assets/branding/` |
| `npm run check:design -- --upstream` | Daily, as the workflow job `Design system drift` | Reports, as a warning, the files that have changed in the Marketplace repository since the recorded commit. It never fails a build or a deployment |
| `npm run check:design -- --update` | By hand, when drift is reported | Takes the current files, updates `SOURCE.json`. Review the site in both themes, run `npm run verify`, commit |

Also shared, by copying the Marketplace site's code rather than by a check:
the theme script in `layouts/Base.astro` (the `dp-theme` storage key), the
interaction script `scripts/ui.ts`, `styles/global.css` (fonts, Tailwind
utilities mapped onto the tokens), and the Astro wrappers of shared
components (`Button`, `Card`, `Badge`, `Notice`, `Section`, `CodeBlock`,
`EmptyState`, `ErrorState`, `LanguageSwitcher`, `ThemeSwitcher`,
`DocsSidebar`, `GitHubMark`). Keep them close to their originals.

## Brand

`src/assets/brand/` holds byte-identical copies of
`.github/assets/branding/` (`icon.svg`, `icon.png`, `wordmark.svg`,
`wordmark-dark.svg`); `npm run check:design` compares them. Never edit,
recolour or redraw them. The social sharing image `src/assets/og.png` is
rendered by `tools/make-og.mjs` from the unmodified dark wordmark.

## The two sites together

- The header has the same anatomy on both sites: wordmark, main links,
  language switcher, theme switcher, GitHub. The Marketplace adds the tag
  "Marketplace" next to the wordmark; the product site has none.
- "Plugins" in this site's navigation, the hero's second button, the
  Marketplace section and the footer lead to the Marketplace site **in the
  same language** (`marketplace(lang)` in `src/data/site.ts`). Such a link
  shows an arrow and tells assistive technology that it opens the
  Marketplace site.
- Both footers end with the same trademark notice.
- Both sites store the visitor's theme and language under the same keys
  (`dp-theme`, `dp-lang`). They are on one origin,
  `knittingcode.github.io`, and browser storage is per origin, so a choice
  made on one site is honoured by the other.

## Product site components

Classes that only this site needs are in
[`src/styles/site.css`](src/styles/site.css). They read the tokens and never
write a colour, a size or a duration of their own.

| Component | Class | Notes |
| --- | --- | --- |
| External navigation link | `.nav-link-external` | A main link that leaves for the Marketplace |
| Feature row | `.feature`, `.feature-title`, `.feature-text` | Icon tile with a heading and a sentence |
| Checklist | `.check-list` | A list with check marks |
| Two parts | `.parts`, `.parts-link` | The agent and the app, joined by how they talk |
| Specification list | `.spec-list` | Label and value pairs inside a card |
| Module grid | `.module-grid`, `.module` | The built-in modules, dense and quiet |
| Panel | `.panel` | A wide call-out surface with the hero's glow |
| Screenshot | `.shot`, `.shot-frame`, `.is-placeholder` | `Screenshot.astro`: a real image, or a dashed frame that says there is none |
| Release bar | `.release-bar` | Version, channel, date and notes of a release |
| Download asset | `.asset-head`, `.asset-file`, `.hash` | One installer with its size, file name, checksum and signing |
| File list | `.file-list` | File names and what they are |

Rules that go beyond the shared ones:

- **Nothing pretends to be the application.** The hero's key grid is a
  drawing of a device, not of the app; a missing screenshot is a labelled
  placeholder, never a mock-up.
- **State is said in words.** Supported, layout only, pre-release,
  unsigned, experimental: each is a badge with text, never a colour alone.
- **Warnings are for facts the visitor must not miss**: the pre-release
  status, an unsigned installer, the absence of a security review.
