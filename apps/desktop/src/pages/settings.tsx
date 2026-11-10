import { Trash2 } from "lucide-react"
import { useState } from "react"

import { EmptyState, ErrorNotice, Loading, Page } from "@/components/page"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useAsync } from "@/hooks/use-async"
import { api, errorMessage } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { RESOURCE_KIND_LABELS, osLabel } from "@/lib/format"
import type { ResourceKind } from "@/lib/types"

const KINDS = Object.keys(RESOURCE_KIND_LABELS) as ResourceKind[]

export function SettingsPage() {
  const { revision, refresh } = useAppContext()
  const { state, reload } = useAsync(() => api.getMachineConfig(), String(revision))
  const [key, setKey] = useState("")
  const [kind, setKind] = useState<ResourceKind>("directory")
  const [value, setValue] = useState("")
  const [error, setError] = useState<string | null>(null)

  const run = async (operation: () => Promise<unknown>) => {
    setError(null)
    try {
      await operation()
      reload()
      refresh()
    } catch (e) {
      setError(errorMessage(e))
    }
  }

  const submit = (event: React.SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault()
    void run(async () => {
      await api.setResourceBinding(key, kind, value)
      setKey("")
      setValue("")
    })
  }

  return (
    <Page title="Settings" description="Settings that apply to this computer only">
      {error && <ErrorNotice error={error} />}
      <Card>
        <CardHeader>
          <CardTitle>Machine resource bindings</CardTitle>
          <CardDescription>
            Profiles refer to logical resource keys. These bindings give each key a concrete value
            on this computer
            {state.status === "ready" ? ` (${osLabel(state.data.os)})` : ""} and are never part of a
            profile.
          </CardDescription>
        </CardHeader>
        <CardContent className="grid gap-6">
          {state.status === "loading" && <Loading />}
          {state.status === "error" && <ErrorNotice error={state.error} />}
          {state.status === "ready" &&
            (state.data.bindings.length === 0 ? (
              <EmptyState title="No resources bound on this machine" />
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Resource key</TableHead>
                    <TableHead>Kind</TableHead>
                    <TableHead>Value</TableHead>
                    <TableHead className="w-0" />
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {state.data.bindings.map((binding) => (
                    <TableRow key={binding.key}>
                      <TableCell className="font-mono">{binding.key}</TableCell>
                      <TableCell>{RESOURCE_KIND_LABELS[binding.kind]}</TableCell>
                      <TableCell className="font-mono break-all whitespace-normal select-text">
                        {binding.value}
                      </TableCell>
                      <TableCell>
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label={`Remove ${binding.key}`}
                          onClick={() => void run(() => api.removeResourceBinding(binding.key))}
                        >
                          <Trash2 />
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            ))}

          <form className="grid max-w-xl gap-4" onSubmit={submit}>
            <div className="grid grid-cols-[1fr_12rem] gap-4">
              <div className="grid gap-2">
                <Label htmlFor="binding-key">Resource key</Label>
                <Input
                  id="binding-key"
                  value={key}
                  onChange={(e) => {
                    setKey(e.target.value)
                  }}
                  placeholder="shop.repository"
                  required
                />
              </div>
              <div className="grid gap-2">
                <Label htmlFor="binding-kind">Kind</Label>
                <Select
                  value={kind}
                  onValueChange={(next) => {
                    setKind(next as ResourceKind)
                  }}
                >
                  <SelectTrigger id="binding-kind" className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {KINDS.map((option) => (
                      <SelectItem key={option} value={option}>
                        {RESOURCE_KIND_LABELS[option]}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="grid gap-2">
              <Label htmlFor="binding-value">Value</Label>
              <Input
                id="binding-value"
                value={value}
                onChange={(e) => {
                  setValue(e.target.value)
                }}
                placeholder="Path, application name, URL, container name…"
                required
              />
            </div>
            <div>
              <Button type="submit" disabled={!key.trim() || !value.trim()}>
                Save binding
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Security</CardTitle>
          <CardDescription>
            The storage layer supports SQLCipher database encryption when built with the
            <code className="mx-1">sqlcipher</code> feature. This build stores the database
            unencrypted; password setup and encrypted import and export are planned.
          </CardDescription>
        </CardHeader>
      </Card>
    </Page>
  )
}
