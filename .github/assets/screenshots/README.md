# Screenshots

Product screenshots for the repository README. This folder holds real
screenshots of the released app only: no mockups, no generated images.

## Planned set, in README order

| File | Shows |
| --- | --- |
| `01-dashboard.png` | Main dashboard |
| `02-device.png` | Stream Deck device view and layout editor |
| `03-profiles-projects.png` | Profiles and projects |
| `04-action-editor.png` | Action editor |
| `05-workflow-editor.png` | Workflow editor |
| `06-work-session.png` | Work Sessions |
| `07-settings-modules.png` | Settings and modules |

Number the files in the order they appear; add a `-dark` variant
(`01-dashboard-dark.png`) when a screenshot exists in both themes.

## Format

- PNG, captured on macOS at 2× (Retina) from the app's default window of
  1120 × 740 points (2240 × 1480 pixels, about 3:2). Keep every image in the
  set at the same size.
- Demonstration data only (`make seed`), never a real configuration: no real
  project names, paths, hosts or credentials.
- Window only, without the desktop or other applications (`screencapture -l
  <window id>`), cropped to the window shadow.
- Compress losslessly before committing (for example `oxipng -o 4` or
  `pngcrush`); aim for under 500 KB per image.

## Light and dark themes

GitHub shows the README in the reader's theme. When both variants exist, use
a `<picture>` element so each reader sees the matching one:

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset=".github/assets/screenshots/01-dashboard-dark.png" />
  <img src=".github/assets/screenshots/01-dashboard.png" alt="DeckPilot dashboard" width="800" />
</picture>
```

## Adding the gallery to the README

The README has an HTML comment, "App preview", after the feature sections.
Replace it with an `## App preview` section: the dashboard large (800 px
wide), then the other screenshots, each under a short heading that names the
feature. Add only images that exist in this folder.
