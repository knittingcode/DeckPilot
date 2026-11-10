import { History } from "lucide-react"

import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { formatTimestamp } from "@/lib/format"

/** Changes that database upgrades made to stored data. Hidden when there are none. */
export function MigrationNotesCard() {
  const { profiles } = useAppContext()
  const { state } = useAsync(() => api.migrationNotes(), "migration-notes")
  if (state.status !== "ready" || state.data.length === 0) return null
  const profileName = (id: string | null) =>
    profiles.profiles.find((p) => p.id === id)?.name ?? "a deleted profile"
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <History className="size-4" /> Data upgrade notes
        </CardTitle>
        <CardDescription>
          Upgrading the database changed the following stored data. Review the affected items.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <ul className="grid gap-2 text-sm">
          {state.data.map((note, index) => (
            <li key={index} className="grid gap-0.5">
              <span>{note.message}</span>
              <span className="text-xs text-muted-foreground">
                {note.profile ? `Profile ${profileName(note.profile)} · ` : ""}
                {formatTimestamp(note.createdAt)}
              </span>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  )
}
