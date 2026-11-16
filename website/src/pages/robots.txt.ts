import type { APIRoute } from 'astro';
import { withBase } from '@/lib/paths';

/**
 * Served at <base>/robots.txt. Crawlers only read robots.txt at the root of
 * a host, which this project site does not own, so the file is a courtesy:
 * the sitemap is also linked from every page head, and pages that must stay
 * out of indexes carry a robots meta tag.
 */
export const GET: APIRoute = ({ site }) => {
  const lines = ['User-agent: *', 'Allow: /', '', `Sitemap: ${new URL(withBase('/sitemap-index.xml'), site).href}`];
  return new Response(`${lines.join('\n')}\n`, { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
