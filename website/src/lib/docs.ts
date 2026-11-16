import { getCollection, type CollectionEntry } from 'astro:content';
import type { Lang } from '@/i18n';

export type Doc = CollectionEntry<'docs'>;
export const DOC_GROUPS = ['start', 'design', 'automate', 'run', 'help'] as const;
export type DocGroup = (typeof DOC_GROUPS)[number];

/**
 * The DeckPilot commit the user documentation (src/content/docs) was
 * checked against. Update it together with the documentation.
 */
export const DOCS_VERIFIED_COMMIT = '18475f2184d3fa3e6fe4659a78c2e8e6919fd1b6';

/** 'en/installation' -> 'installation'. */
export function slugOf(doc: Doc): string {
  return doc.id.split('/').slice(1).join('/');
}

/** The documentation pages of a language, in sidebar order. */
export async function docsOf(lang: Lang): Promise<Doc[]> {
  const all = await getCollection('docs', (doc) => doc.id.startsWith(`${lang}/`));
  return all.sort((a, b) => a.data.order - b.data.order);
}

export function grouped(docs: Doc[]): { group: DocGroup; docs: Doc[] }[] {
  return DOC_GROUPS.map((group) => ({ group, docs: docs.filter((doc) => doc.data.group === group) })).filter(
    (entry) => entry.docs.length > 0,
  );
}
