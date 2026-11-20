import { DialogProvider } from "@/components/dialogs"
import { ThemeProvider } from "@/components/theme-provider"
import { TooltipProvider } from "@/components/ui/tooltip"

import { DialogShowcase } from "./dialog-showcase"

/**
 * Development only: the Dialog Showcase outside the app shell, for review in
 * a plain browser (`/?showcase=dialogs`). Loaded by main.tsx only
 * when `import.meta.env.DEV`.
 */
export function StandaloneShowcase() {
  return (
    <ThemeProvider>
      <TooltipProvider>
        <DialogProvider>
          <main className="mx-auto grid max-w-4xl gap-8 p-8">
            <DialogShowcase />
          </main>
        </DialogProvider>
      </TooltipProvider>
    </ThemeProvider>
  )
}
