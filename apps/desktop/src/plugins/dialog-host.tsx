// Plugin dialogs (plugin API 1.5, docs/dialogs.md "Plugin dialog API").
//
// A plugin never ships UI: it asks the agent for a standard dialog
// (`dialog.open`), the agent validates the request and sends it here as a
// `dialogOpened` plugin event (main window only), and this host renders it
// with DeckPilot's own templates and answers with structured values. The
// agent checks the answer again before the plugin sees it.
//
// Rules:
// - Mounted only in the main window, inside the `DialogProvider` (so only
//   while the configuration is unlocked), and only in builds with plugins.
//   While it is mounted the app announces plugin dialogs to the agent; when
//   it unmounts it stops, and the agent cancels what is open (`appClosed`).
// - One plugin dialog at a time, FIFO; a new one waits while a DeckPilot
//   dialog of the service is open. A dialog the agent closes (timeout, the
//   plugin stopped) is taken down unanswered, open or queued.
// - Every plugin dialog names its plugin (a visible requester line with a
//   generic plugin icon), never shows DeckPilot's lock or shield icons and is
//   never blocking: Cancel, Escape and the close button always cancel.
// - Text is plain text: titles, messages and labels are rendered as text
//   nodes, never as markup.

import { listen } from "@tauri-apps/api/event"
import { Puzzle } from "lucide-react"
import { type ReactNode, useEffect, useId, useRef, useState } from "react"

import {
  AlertDialog,
  ConfirmDialog,
  FieldError,
  FormDialog,
  SelectionDialog,
} from "@/components/dialogs"
import { useServiceDialogOpen } from "@/components/dialogs/use-dialogs"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { Textarea } from "@/components/ui/textarea"
import { Trans, useTranslation } from "@/i18n"

import { toPluginError } from "./api"
import { PLUGIN_EVENT, answerPluginDialog, setPluginDialogsReady } from "./client"
import {
  DEFAULT_MAX_LENGTH,
  type Draft,
  MAX_ANSWER_BYTES,
  fieldValue,
  initialDraft,
  tooLongText,
} from "./dialog-values"
import type {
  PluginDialog,
  PluginDialogAnswer,
  PluginDialogField,
  PluginDialogValue,
  PluginEvent,
} from "./types"

/** Mounts the plugin dialog host: renders the agent's plugin dialogs, one at a time. */
export function PluginDialogHost() {
  const [queue, setQueue] = useState<PluginDialog[]>([])
  // The dialog on screen stays there even if a DeckPilot dialog is opened
  // meanwhile (that one nests on top); only the next one waits.
  const [shown, setShown] = useState<string | null>(null)
  const serviceOpen = useServiceDialogOpen()

  useEffect(() => {
    let active = true
    let stop: (() => void) | undefined
    void listen<PluginEvent>(PLUGIN_EVENT, (event) => {
      if (!active) return
      const payload = event.payload
      if (payload.type === "dialogOpened") {
        const dialog = payload.dialog
        setQueue((q) => (q.some((d) => d.id === dialog.id) ? q : [...q, dialog]))
      } else if (payload.type === "dialogClosed") {
        setQueue((q) => q.filter((d) => d.id !== payload.dialog))
      }
    }).then((unlisten) => {
      if (!active) {
        unlisten()
        return
      }
      stop = unlisten
      // Listening first, so no dialog announced afterwards is missed.
      void setPluginDialogsReady(true).catch(() => undefined)
    })
    return () => {
      active = false
      stop?.()
      void setPluginDialogsReady(false).catch(() => undefined)
    }
  }, [])

  const head = queue[0]
  const showing = head !== undefined && (shown === head.id || !serviceOpen)
  // Remembered while rendering (React's "adjusting state" pattern), so the
  // dialog stays once it is on screen.
  if (showing && shown !== head.id) setShown(head.id)

  if (!head || !showing) return null
  return (
    <PluginDialogView
      key={head.id}
      dialog={head}
      onDone={() => {
        setQueue((q) => q.filter((d) => d.id !== head.id))
      }}
    />
  )
}

/** Who asks: the plugin's name from its installed manifest, always visible. */
function Requester({ name }: { name: string }) {
  return (
    <div
      className="flex items-center gap-2 text-sm text-muted-foreground"
      data-dialog-part="requester"
    >
      <span
        className="flex size-6 shrink-0 items-center justify-center rounded-md bg-muted text-foreground"
        aria-hidden
      >
        <Puzzle className="size-3.5" />
      </span>
      <span>
        <Trans
          i18nKey="plugins:dialog.requestedBy"
          values={{ name }}
          components={{ name: <span className="font-medium text-foreground" /> }}
        />
      </span>
    </div>
  )
}

/** Sends an answer; resolves when the agent took it or the dialog is gone anyway. */
async function send(dialog: PluginDialog, answer: PluginDialogAnswer): Promise<void> {
  try {
    await answerPluginDialog(dialog.id, dialog.plugin, answer)
  } catch (error) {
    const refused = toPluginError(error)
    // Answered elsewhere, timed out or its plugin stopped: nothing to keep open.
    if (refused.reason === "dialogClosed") return
    throw refused
  }
}

function PluginDialogView({ dialog, onDone }: { dialog: PluginDialog; onDone: () => void }) {
  const { t } = useTranslation()
  const settled = useRef(false)
  const { spec } = dialog
  const description = spec.message
  const requester = <Requester name={dialog.pluginName} />

  const confirm = async (answer: PluginDialogAnswer) => {
    if (settled.current) return
    await send(dialog, answer)
    settled.current = true
  }
  // Cancel, Escape, the close button, and the templates' close after a
  // confirmed answer (ignored then).
  const close = () => {
    if (!settled.current) {
      settled.current = true
      void send(dialog, { outcome: "cancelled", reason: "user" }).catch(() => undefined)
    }
    onDone()
  }

  switch (spec.type) {
    case "confirm":
      return (
        <ConfirmDialog
          open
          onOpenChange={(open) => {
            if (!open) close()
          }}
          destructive={spec.destructive === true}
          icon={spec.destructive === true ? undefined : Puzzle}
          title={spec.title}
          description={description}
          confirmLabel={spec.confirmLabel ?? t("common:actions.ok")}
          cancelLabel={spec.cancelLabel ?? t("common:actions.cancel")}
          onConfirm={() => confirm({ outcome: "confirmed" })}
        >
          {requester}
        </ConfirmDialog>
      )
    case "alert":
      return (
        <AlertDialog
          open
          onClose={close}
          onAcknowledge={() => {
            void confirm({ outcome: "confirmed" })
              .catch(() => undefined)
              .finally(() => {
                settled.current = true
                onDone()
              })
          }}
          title={spec.title}
          description={description}
        >
          {requester}
        </AlertDialog>
      )
    case "selection":
      return (
        <SelectionDialog
          open
          onClose={close}
          icon={Puzzle}
          title={spec.title}
          description={description}
          options={spec.options ?? []}
          submitLabel={spec.confirmLabel ?? t("dialogs:selection.choose")}
          cancelLabel={spec.cancelLabel ?? t("common:actions.cancel")}
          onSelect={(selected) => confirm({ outcome: "confirmed", selected })}
        >
          {requester}
        </SelectionDialog>
      )
    case "form":
      return (
        <PluginForm dialog={dialog} requester={requester} onClose={close} onConfirm={confirm} />
      )
  }
}

function PluginForm({
  dialog,
  requester,
  onClose,
  onConfirm,
}: {
  dialog: PluginDialog
  requester: ReactNode
  onClose: () => void
  onConfirm: (answer: PluginDialogAnswer) => Promise<void>
}) {
  const { t } = useTranslation()
  const fields = dialog.spec.fields ?? []
  const [draft, setDraft] = useState<Draft>(() => initialDraft(fields))
  const checked = fields.map((field) => ({ field, ...fieldValue(field, draft[field.id]) }))
  const canSubmit = checked.every((c) => c.error === undefined && c.missing !== true)

  return (
    <FormDialog
      open
      onClose={onClose}
      icon={Puzzle}
      title={dialog.spec.title}
      description={dialog.spec.message}
      submitLabel={dialog.spec.confirmLabel ?? t("common:actions.ok")}
      cancelLabel={dialog.spec.cancelLabel ?? t("common:actions.cancel")}
      canSubmit={canSubmit}
      onSubmit={async () => {
        const values: Record<string, PluginDialogValue> = {}
        for (const c of checked) if (c.value !== undefined) values[c.field.id] = c.value
        if (new TextEncoder().encode(JSON.stringify(values)).length > MAX_ANSWER_BYTES) {
          throw new Error(tooLongText())
        }
        await onConfirm({ outcome: "confirmed", values })
      }}
    >
      {requester}
      {checked.map(({ field, error }) => (
        <FieldRow
          key={field.id}
          field={field}
          value={draft[field.id]}
          error={error}
          onChange={(value) => {
            setDraft((d) => ({ ...d, [field.id]: value }))
          }}
        />
      ))}
    </FormDialog>
  )
}

function FieldRow({
  field,
  value,
  error,
  onChange,
}: {
  field: PluginDialogField
  value: string | boolean | undefined
  error: string | undefined
  onChange: (value: string | boolean) => void
}) {
  const { t } = useTranslation()
  const id = useId()
  const describedBy =
    [field.description ? `${id}-help` : null, error ? `${id}-error` : null]
      .filter(Boolean)
      .join(" ") || undefined
  const common = {
    id,
    "aria-describedby": describedBy,
    "aria-invalid": error ? true : undefined,
    "aria-required": field.required || undefined,
  }
  const text = typeof value === "string" ? value : ""

  let input
  switch (field.kind) {
    case "text":
      input = field.multiline ? (
        <Textarea
          {...common}
          value={text}
          maxLength={field.maxLength ?? DEFAULT_MAX_LENGTH}
          onChange={(e) => {
            onChange(e.target.value)
          }}
        />
      ) : (
        <Input
          {...common}
          value={text}
          maxLength={field.maxLength ?? DEFAULT_MAX_LENGTH}
          autoComplete="off"
          onChange={(e) => {
            onChange(e.target.value)
          }}
        />
      )
      break
    case "integer":
      input = (
        <Input
          {...common}
          value={text}
          inputMode="numeric"
          autoComplete="off"
          onChange={(e) => {
            onChange(e.target.value)
          }}
        />
      )
      break
    case "lines":
      input = (
        <Textarea
          {...common}
          value={text}
          placeholder={t("plugins:dialog.onePerLine")}
          onChange={(e) => {
            onChange(e.target.value)
          }}
        />
      )
      break
    case "boolean":
      input = (
        <Switch
          {...common}
          checked={value === true}
          onCheckedChange={(next) => {
            onChange(next)
          }}
        />
      )
      break
    case "choice":
      input = (
        <Select
          value={text}
          onValueChange={(next) => {
            onChange(next)
          }}
        >
          <SelectTrigger {...common} className="w-full">
            <SelectValue placeholder={t("plugins:dialog.choosePlaceholder")} />
          </SelectTrigger>
          <SelectContent>
            {(field.options ?? []).map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )
      break
  }

  return (
    <div className="grid gap-1.5" data-field={field.id}>
      <Label htmlFor={id}>
        {field.label}
        {field.required && (
          <span className="text-muted-foreground"> {t("plugins:dialog.required")}</span>
        )}
      </Label>
      {input}
      {field.description && (
        <p id={`${id}-help`} className="text-xs text-muted-foreground">
          {field.description}
        </p>
      )}
      <FieldError id={`${id}-error`} message={error} />
    </div>
  )
}
