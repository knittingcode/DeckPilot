import { ArrowRightLeft, Download, Upload } from "lucide-react"

import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { api } from "@/lib/api"
import { toastFailure } from "@/lib/failure"

/**
 * Encrypted export and import of the portable configuration. Both run in
 * native windows of their own (`portability.export`, `portability.import`).
 */
export function PortabilityCard() {
  const open = (id: string) => {
    api.openWindow(id).catch((e: unknown) => {
      toastFailure("The window did not open", e)
    })
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ArrowRightLeft className="size-4" /> Export and import
        </CardTitle>
        <CardDescription>
          Move profiles, projects, actions, workflows, layouts, module and application settings to
          another computer in an encrypted file — and keep one as your backup. Paths and other
          settings of this computer, keys and passwords are not included.
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          onClick={() => {
            open("portability.export")
          }}
        >
          <Download /> Export…
        </Button>
        <Button
          variant="outline"
          onClick={() => {
            open("portability.import")
          }}
        >
          <Upload /> Import…
        </Button>
      </CardContent>
    </Card>
  )
}
