import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { z } from 'astro/zod';

/**
 * User documentation: src/content/docs/<lang>/<slug>.md. Both languages
 * must have the same slugs (tests/check-i18n.mjs compares the built routes).
 */
const docs = defineCollection({
  loader: glob({ pattern: '{en,pl}/*.md', base: './src/content/docs' }),
  schema: z.object({
    title: z.string().min(1).max(60),
    description: z.string().min(50).max(160),
    group: z.enum(['start', 'design', 'automate', 'run', 'help']),
    order: z.number().int().positive(),
    /** Shown above the page when the feature is not in released builds. */
    experimental: z.boolean().default(false),
  }),
});

export const collections = { docs };
