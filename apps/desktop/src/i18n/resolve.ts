import { FALLBACK_LANGUAGE, type Language, SUPPORTED_LANGUAGES } from "./config"

/**
 * The language to show for the user's preferred languages, most preferred
 * first (BCP 47 tags such as `pl-PL`, `en-GB`, `zh-Hant-TW`).
 *
 * Each preferred language is tried in turn: a supported tag that matches
 * exactly, then a supported tag for its language alone (`pl-PL` → `pl`);
 * only when neither matches is the next preferred language tried. When no
 * preferred language is supported the result is English. Only the user's
 * language list decides: never the IP address, time zone or region.
 */
export function resolveLocale<L extends string = Language>(
  preferred: readonly string[],
  supported: readonly L[] = SUPPORTED_LANGUAGES as unknown as readonly L[],
  fallback: L = FALLBACK_LANGUAGE as L,
): L {
  const byTag = new Map(supported.map((tag) => [normalize(tag), tag]))
  for (const candidate of preferred) {
    const tag = normalize(candidate)
    if (!tag) continue
    const exact = byTag.get(tag)
    if (exact) return exact
    const language = byTag.get(tag.split("-")[0] ?? "")
    if (language) return language
  }
  return fallback
}

/** `pl_PL.UTF-8` → `pl-pl`: case-insensitive BCP 47 for comparisons. */
function normalize(tag: string): string {
  return (tag.split(/[.@]/)[0] ?? "").trim().replaceAll("_", "-").toLowerCase()
}
