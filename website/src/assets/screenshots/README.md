# Screenshots

Images of the gallery at `/{en,pl}/screenshots/`. This folder is empty until
real screenshots exist: the gallery then shows placeholders that say so.

Only real screenshots of the application belong here, made as described in
[`.github/assets/screenshots/README.md`](../../../../.github/assets/screenshots/README.md)
(a build of current `main` with demonstration data, window only, 2240 × 1480
pixels). No mock-ups and no generated images.

## Adding one

Save the image under the file name the gallery expects
([`src/data/screenshots.ts`](../../data/screenshots.ts)):

| File | Shows |
| --- | --- |
| `01-dashboard.png` | Dashboard |
| `02-device.png` | Layout Designer |
| `03-profiles-projects.png` | Profiles and projects |
| `04-action-editor.png` | Action editor |
| `05-workflow-editor.png` | Workflow editor |
| `06-work-session.png` | Work Session |
| `07-settings-modules.png` | Settings and modules |

Add `-dark` before the extension (`01-dashboard-dark.png`) for the dark
theme's variant; without it the light image is shown in both themes. PNG,
JPEG and WebP are accepted. The build makes the AVIF and WebP sizes itself,
so commit the full-size original.

Then check the text alternative and caption of that screenshot in
`src/i18n/en.ts` and `src/i18n/pl.ts` (`screenshots.items`): they must
describe what the image really shows.
