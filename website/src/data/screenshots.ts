import type { ImageMetadata } from 'astro';
import type { Dictionary } from '@/i18n';

/**
 * The screenshot gallery. The set and its file names follow the plan for
 * the repository's README (.github/assets/screenshots/README.md).
 *
 * To publish a screenshot, put the image in src/assets/screenshots/ under
 * the `file` name below (png, jpg or webp), and optionally a second one
 * with the suffix `-dark` for the dark theme:
 *
 *   src/assets/screenshots/01-dashboard.png
 *   src/assets/screenshots/01-dashboard-dark.png
 *
 * Nothing else changes: the gallery shows the image instead of the
 * placeholder on the next build. Only real screenshots of the application
 * belong there (see src/assets/screenshots/README.md).
 */
export type ScreenshotId = keyof Dictionary['screenshots']['items'];

export const SCREENSHOTS: { id: ScreenshotId; file: string }[] = [
  { id: 'dashboard', file: '01-dashboard' },
  { id: 'device', file: '02-device' },
  { id: 'profiles', file: '03-profiles-projects' },
  { id: 'actionEditor', file: '04-action-editor' },
  { id: 'workflowEditor', file: '05-workflow-editor' },
  { id: 'workSession', file: '06-work-session' },
  { id: 'settings', file: '07-settings-modules' },
];

const files = import.meta.glob<{ default: ImageMetadata }>('/src/assets/screenshots/*.{png,jpg,jpeg,webp}', { eager: true });

function find(name: string): ImageMetadata | undefined {
  const entry = Object.entries(files).find(([path]) => path.replace(/^.*\//, '').replace(/\.[a-z]+$/, '') === name);
  return entry?.[1].default;
}

/** The images of a screenshot, or undefined while it has not been published. */
export function imagesOf(file: string): { light: ImageMetadata; dark?: ImageMetadata } | undefined {
  const light = find(file);
  if (!light) return undefined;
  return { light, dark: find(`${file}-dark`) };
}

export function anyMissing(): boolean {
  return SCREENSHOTS.some((shot) => !imagesOf(shot.file));
}
