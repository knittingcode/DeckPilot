import { clsx, type ClassValue } from "clsx"
import { extendTailwindMerge } from "tailwind-merge"

// `text-caption` is a font size of the theme (index.css); without this,
// merging it with a text colour would drop it as if it were a colour.
const twMerge = extendTailwindMerge({
  extend: { theme: { text: ["caption"] } },
})

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
