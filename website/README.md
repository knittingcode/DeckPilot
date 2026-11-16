# DeckPilot website

The product website of DeckPilot: <https://knittingcode.github.io/deckpilot/>.
A static site (Astro, TypeScript) in English and Polish, deployed to GitHub
Pages by [`.github/workflows/website.yml`](../.github/workflows/website.yml).

It is a separate project inside the repository. It has its own
`package.json` and lockfile (npm), is not part of the pnpm workspace of the
desktop app, and takes no part in building, signing or releasing the
application. It hosts no installer and no update file: downloads are links
to GitHub Releases, and the application's updater never reads this site.

Its sister site is the Plugin Marketplace,
<https://knittingcode.github.io/deckpilot-plugins/>
([`knittingcode/deckpilot-plugins`](https://github.com/knittingcode/deckpilot-plugins)).
Both use one design system: see [DESIGN.md](DESIGN.md).

## Working on it

Node.js 22.12 or newer (CI uses the version in `.node-version`).

```sh
cd website
npm ci
npm run dev        # http://localhost:4321/deckpilot/
npm run verify     # everything CI runs
```

| Script | Does |
| --- | --- |
| `npm run check:design` | The shared stylesheets and brand files are unmodified copies ([DESIGN.md](DESIGN.md)); `-- --upstream` also compares with the Marketplace repository, `-- --update` takes its current files |
| `npm run check` | Type check (`astro check`), including that Polish has every key English has |
| `npm run build` | Builds `dist/`, reading the published releases from GitHub |
| `npm run build:fixtures` | Builds `dist-fixtures/` from invented release lists (tests only) |
| `npm run test:unit` | The rules that choose the release and its installers |
| `npm run test:static` | No hardcoded version in `src/`; every internal link and anchor; the same routes in both languages, `hreflang`, canonical, no link into the other language |
| `npm run test:browser` | Playwright: navigation, language and theme switching, mobile, keyboard, downloads against GitHub, documentation, SEO metadata, 404, and axe (WCAG 2.2 AA) in both themes |
| `npm run test:live` | The same browser suite against the deployed site (`SITE_URL`, default the production URL) |
| `node tests/check-links.mjs dist --external` | Also requests every external link (not part of CI) |

The browser tests of the download page use the network: they compare the
page with GitHub's API and request every download link.

## Routes

| Route | Page |
| --- | --- |
| `/` | Forwards to the visitor's language (`/en/` or `/pl/`) |
| `/{en,pl}/` | Homepage |
| `/{en,pl}/download/` | The latest published release: installers, checksums, signing |
| `/{en,pl}/screenshots/` | Screenshot gallery |
| `/{en,pl}/docs/`, `/{en,pl}/docs/<page>/` | User documentation |
| `/404.html`, `/{en,pl}/404/` | Not found |

Plugins and plugin development are not pages of this site: navigation
links lead to the Marketplace site, in the same language.

## Layout

```text
website/
  astro.config.ts          site URL, base path, sitemap, code highlighting
  DESIGN.md                the shared design system and this site's additions
  src/
    styles/shared/         tokens.css, base.css, components.css: vendored, never edited here
    styles/site.css        classes only this site needs
    styles/global.css      fonts, Tailwind mapped onto the tokens
    components/            Astro components (header, footer, cards, screenshot, ...)
    layouts/Base.astro     document head, SEO, theme, header and footer
    i18n/                  en.ts (reference), pl.ts (same shape), helpers
    data/                  links, navigation, the screenshot list
    lib/release-model.ts   which release and which files the site shows (pure, unit-tested)
    lib/releases.ts        reads the releases from GitHub at build time
    content/docs/{en,pl}   user documentation (Markdown)
    assets/screenshots/    real screenshots, when they exist
    pages/                 routes
  tests/                   static checks, unit tests, Playwright suites, fixtures, server
  tools/                   design system check, social image
```

## Languages

English is the default and the reference. `src/i18n/pl.ts` has the type of
`en.ts`, so a missing or extra Polish key fails the type check. Every page
exists in both languages (checked on the built output), links its
counterpart with `hreflang`, and never links into the other language except
through the language switcher. The application's interface is English only,
so names of its screens and buttons stay in English on Polish pages.

To add a documentation page, add `src/content/docs/en/<slug>.md` and
`src/content/docs/pl/<slug>.md` with the same `group` and `order`.

## Accuracy

The site states what DeckPilot does today, including what it does not do.

- **No version is written by hand.** The release on the homepage badge and
  the download page comes from GitHub's API when the site is built
  (`src/lib/releases.ts`); `tests/check-no-version.mjs` fails if a version
  appears in `src/`.
- **Drafts never appear.** Only releases with `draft: false` and a
  publication date are considered. A stable release is offered before a
  newer pre-release, which is then mentioned.
- **A button exists only for a file the release has**, and only for assets
  served from this repository's release downloads.
- **Signing statements come from the release.** They are read from the
  release's own `release-meta.json`; a platform it does not describe gets
  no claim.
- **The documentation** was written from the application's code and its
  technical documentation at the commit recorded in `DOCS_VERIFIED_COMMIT`
  ([`src/lib/docs.ts`](src/lib/docs.ts)). Update the constant together with
  the pages.
- **Screenshots** are real or absent: [`src/assets/screenshots/README.md`](src/assets/screenshots/README.md)
  says how to add one. Nothing else needs to change.

## How the download page stays current

A build bakes the release into the pages. The workflow builds again on
every push to `main` that touches the website, every day, when started by
hand, and when a release is published. As a last resort the download page
asks GitHub's API in the browser whether a newer release exists and shows a
notice with a link; if that request fails, nothing changes.

A build fails when GitHub's API cannot be read, so a deployment never
replaces a correct page with an empty one.

## Deployment

`.github/workflows/website.yml`:

- pull requests that touch `website/` build and test, and never deploy;
- pushes to `main`, the daily schedule and manual runs build, test and
  deploy `website/dist` to GitHub Pages through the `github-pages`
  environment, which accepts `main` only;
- the workflow uses no secret. Its jobs have `contents: read`; only the
  deploy job has `pages: write` and `id-token: write`, and the job that
  restarts the workflow after a release has `actions: write` and runs no
  repository code.

## License

Apache-2.0, like the rest of this repository. The DeckPilot logo files in
`src/assets/brand/` are unmodified copies of `.github/assets/branding/`.
Fonts (Inter, JetBrains Mono) are bundled from Fontsource under the SIL
Open Font License; icons are from Lucide (ISC). The GitHub mark is from
Octicons (MIT).
