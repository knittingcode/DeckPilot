import { Trash2 } from "lucide-react"
import { useState } from "react"

import { EmptyState, ErrorNotice, Page } from "@/components/page"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Textarea } from "@/components/ui/textarea"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { formatTimestamp } from "@/lib/format"

export function ProfilesPage() {
  const { profiles, refresh } = useAppContext()
  const [name, setName] = useState("")
  const [description, setDescription] = useState("")
  const [error, setError] = useState<string | null>(null)

  const run = async (operation: () => Promise<unknown>) => {
    setError(null)
    try {
      await operation()
      refresh()
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  const create = (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    void run(async () => {
      await api.createProfile(name, description)
      setName("")
      setDescription("")
    })
  }

  return (
    <Page title="Profiles" description="Independent sets of actions, workflows and projects">
      {error && <ErrorNotice error={error} />}
      <Card>
        <CardHeader>
          <CardTitle>All profiles</CardTitle>
          <CardDescription>
            The active profile is used by the dashboard, actions and projects pages.
          </CardDescription>
        </CardHeader>
        <CardContent>
          {profiles.profiles.length === 0 ? (
            <EmptyState title="No profiles yet">Create your first profile below.</EmptyState>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Last modified</TableHead>
                  <TableHead className="w-0" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {profiles.profiles.map((profile) => {
                  const active = profile.id === profiles.activeProfileId
                  return (
                    <TableRow key={profile.id}>
                      <TableCell className="font-medium">
                        {profile.name} {active && <Badge className="ml-2">Active</Badge>}
                      </TableCell>
                      <TableCell className="text-muted-foreground">
                        {formatTimestamp(profile.updatedAt)}
                      </TableCell>
                      <TableCell className="flex justify-end gap-2">
                        <Button
                          variant="outline"
                          size="sm"
                          disabled={active}
                          onClick={() => void run(() => api.setActiveProfile(profile.id))}
                        >
                          Make active
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label={`Delete ${profile.name}`}
                          onClick={() => void run(() => api.deleteProfile(profile.id))}
                        >
                          <Trash2 />
                        </Button>
                      </TableCell>
                    </TableRow>
                  )
                })}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>New profile</CardTitle>
        </CardHeader>
        <CardContent>
          <form className="grid max-w-lg gap-4" onSubmit={create}>
            <div className="grid gap-2">
              <Label htmlFor="profile-name">Name</Label>
              <Input
                id="profile-name"
                value={name}
                onChange={(e) => {
                  setName(e.target.value)
                }}
                placeholder="Work"
                required
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="profile-description">Description</Label>
              <Textarea
                id="profile-description"
                value={description}
                onChange={(e) => {
                  setDescription(e.target.value)
                }}
                placeholder="Optional"
              />
            </div>
            <div>
              <Button type="submit" disabled={!name.trim()}>
                Create profile
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </Page>
  )
}
