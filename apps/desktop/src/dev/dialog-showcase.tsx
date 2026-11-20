import {
  CircleHelp,
  FolderGit2,
  KeyRound,
  LockKeyhole,
  type LucideIcon,
  Plus,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react"
import { useEffect, useState } from "react"

import {
  AlertDialog,
  ConfirmDialog,
  CredentialDialog,
  DestructiveDialog,
  FieldError,
  FormDialog,
  ModalShell,
  PermissionDialog,
  ProgressDialog,
  SelectionDialog,
  useDialogs,
} from "@/components/dialogs"
import { PasswordField } from "@/components/password-field"
import { SettingsGroup, SettingsRow, SettingsSection } from "@/components/settings-layout"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"

/**
 * Development-only: every dialog template in its states, built from the
 * real shared components (no mockups), for UI reviews. Registered in
 * Settings › Advanced only when `import.meta.env.DEV` (settings/registry.ts),
 * so release builds contain neither the page nor this code.
 *
 * Nothing here talks to the agent: actions are simulated with timers, and
 * the credential example checks against a fixed demo string, never a real
 * password.
 */

const wait = (ms: number) =>
  new Promise<void>((resolve) => {
    setTimeout(resolve, ms)
  })

const LONG =
  "DeckPilot keeps the layouts of every profile in the shared library. Deleting this layout removes it from the profiles Work, Streaming and Coding as well; their keys fall back to the default layout of the device model. Backups made before today still contain it, and an encrypted export restores it together with its pages, icons and the actions its keys run. "

const HEADERS: {
  icon: LucideIcon
  title: string
  description: string
  tone?: "default" | "destructive"
}[] = [
  {
    icon: KeyRound,
    title: "Export configuration",
    description: "Protect the export with its own password.",
  },
  {
    icon: LockKeyhole,
    title: "Unlock DeckPilot",
    description: "Enter your password to unlock DeckPilot.",
  },
  {
    icon: CircleHelp,
    title: "Use this layout in “Work”?",
    description: "The profile shows it from now on.",
  },
  {
    icon: TriangleAlert,
    tone: "destructive",
    title: "Delete “Coding”?",
    description: "Its pages and keys are deleted.",
  },
  {
    icon: ShieldCheck,
    title: "Allow GitHub to use these permissions?",
    description: "Requested by GitHub 0.1.0.",
  },
  {
    icon: Plus,
    title: "Create a layout for Stream Deck MK.2 in the profile “Streaming and recording”",
    description: "A long title wraps; the icon stays at the top.",
  },
  {
    icon: FolderGit2,
    title: "Repository mapping",
    description:
      "A long description wraps naturally and the header grows downward: DeckPilot never truncates or clips dialog text to fit the icon, also with larger system text or longer translations.",
  },
]

type Open =
  | null
  | "confirm"
  | "destructive"
  | "destructive-error"
  | "alert-info"
  | "alert-warning"
  | "alert-error"
  | "form"
  | "form-long"
  | "selection"
  | "selection-long"
  | "permission"
  | "progress"
  | "progress-indeterminate"
  | "credential"

export function DialogShowcase() {
  const [open, setOpen] = useState<Open>(null)
  const close = () => {
    setOpen(null)
  }
  const show = (which: Exclude<Open, null>) => () => {
    setOpen(which)
  }
  const dialogs = useDialogs()

  return (
    <>
      <SettingsSection
        title="Dialog showcase"
        description="Development builds only. Every template of components/dialogs in its states, using the real components. Actions are simulated. DeckPilot is always dark."
      >
        <SettingsGroup>
          <Row label="Confirm" description="Non-destructive confirmation.">
            <Button size="sm" variant="outline" onClick={show("confirm")}>
              Default
            </Button>
          </Row>
          <Row label="Destructive" description="Deletes data; async with busy state.">
            <Button size="sm" variant="outline" onClick={show("destructive")}>
              Default + loading
            </Button>
            <Button size="sm" variant="outline" onClick={show("destructive-error")}>
              Fails (inline error)
            </Button>
          </Row>
          <Row label="Alert" description="Information, warning, recoverable error.">
            <Button size="sm" variant="outline" onClick={show("alert-info")}>
              Info
            </Button>
            <Button size="sm" variant="outline" onClick={show("alert-warning")}>
              Warning
            </Button>
            <Button size="sm" variant="outline" onClick={show("alert-error")}>
              Error
            </Button>
          </Row>
          <Row label="Form" description="Validation, async save, unsaved changes.">
            <Button size="sm" variant="outline" onClick={show("form")}>
              Default + validation
            </Button>
            <Button size="sm" variant="outline" onClick={show("form-long")}>
              Long content
            </Button>
          </Row>
          <Row label="Selection" description="Radio list; filter when long.">
            <Button size="sm" variant="outline" onClick={show("selection")}>
              Short
            </Button>
            <Button size="sm" variant="outline" onClick={show("selection-long")}>
              Long
            </Button>
          </Row>
          <Row label="Permission" description="Host-owned capability consent.">
            <Button size="sm" variant="outline" onClick={show("permission")}>
              Plugin request
            </Button>
          </Row>
          <Row label="Progress" description="Determinate and indeterminate.">
            <Button size="sm" variant="outline" onClick={show("progress")}>
              Determinate
            </Button>
            <Button size="sm" variant="outline" onClick={show("progress-indeterminate")}>
              Indeterminate + cancel
            </Button>
          </Row>
          <Row
            label="Credential"
            description="Password template (demo: “deckpilot” is correct; “agent” simulates an unreachable agent)."
          >
            <Button size="sm" variant="outline" onClick={show("credential")}>
              Authenticate
            </Button>
          </Row>
          <Row
            label="Service"
            description="useDialogs(): two requests are queued, shown one after another."
          >
            <Button
              size="sm"
              variant="outline"
              onClick={() => {
                void dialogs
                  .confirm({ title: "First request", description: "Shown first." })
                  .then((ok) => dialogs.alert({ title: ok ? "Confirmed" : "Cancelled" }))
                void dialogs.destructive({
                  title: "Second request",
                  description: "Queued until the first one is answered.",
                })
              }}
            >
              Queue two
            </Button>
          </Row>
        </SettingsGroup>
      </SettingsSection>

      <SettingsSection
        title="Header comparison"
        description="The shared header with its icon tile: a title and a one-line description fit the tile's height; long text wraps and the header grows."
      >
        <div className="flex flex-wrap items-start gap-4">
          {HEADERS.map((header) => (
            <Frame key={header.title} width={440}>
              <ModalShell
                size="compact"
                icon={header.icon}
                tone={header.tone}
                title={header.title}
                description={header.description}
              />
            </Frame>
          ))}
        </div>
      </SettingsSection>

      <SettingsSection
        title="Native window frames"
        description="The same shell as a native window (presentation “window”), at the narrowest and widest compact width. These previews are inline; the real windows size themselves to this content."
      >
        <div className="flex flex-wrap items-start gap-6">
          <Frame width={360}>
            <ModalShell
              size="compact"
              density="compact"
              icon={LockKeyhole}
              title="DeckPilot is locked"
              description="Enter your password to unlock DeckPilot."
              onSubmit={() => undefined}
              primary={{ label: "Unlock", disabled: true }}
            >
              <PasswordField
                id="showcase-unlock"
                label="Password"
                value=""
                onChange={() => undefined}
                autoComplete="current-password"
                revealable
                error="Incorrect password. Try again."
                reserveError
              />
            </ModalShell>
          </Frame>
          <Frame width={440}>
            <ModalShell
              size="compact"
              icon={KeyRound}
              title="Export configuration"
              description="Protect the export with its own password."
              error="The file could not be written. Choose another folder."
              secondary={{ label: "Cancel" }}
              primary={{ label: "Export", loading: true, loadingLabel: "Exporting…" }}
            >
              <p className="text-sm text-muted-foreground">{LONG}</p>
            </ModalShell>
          </Frame>
        </div>
      </SettingsSection>

      <ConfirmDialog
        open={open === "confirm"}
        onOpenChange={(o) => {
          if (!o) close()
        }}
        destructive={false}
        title="Use this layout in “Work”?"
        description="The profile shows it on the Stream Deck MK.2 from now on."
        confirmLabel="Use layout"
        onConfirm={() => wait(600)}
      />
      <DestructiveDialog
        open={open === "destructive"}
        onOpenChange={(o) => {
          if (!o) close()
        }}
        title="Delete “Coding”?"
        description="Its pages and keys are deleted. This cannot be undone."
        busyLabel="Deleting…"
        onConfirm={() => wait(1500)}
      />
      <DestructiveDialog
        open={open === "destructive-error"}
        onOpenChange={(o) => {
          if (!o) close()
        }}
        title="Delete “Main”?"
        description="Its pages and keys are deleted. This cannot be undone."
        busyLabel="Deleting…"
        onConfirm={async () => {
          await wait(700)
          throw new Error(
            "“Main” is the default layout of a profile. Choose another default first.",
          )
        }}
      />
      <AlertDialog
        open={open === "alert-info"}
        onClose={close}
        title="Layout copied"
        description="“Coding (copy)” is in the shared library."
      />
      <AlertDialog
        open={open === "alert-warning"}
        onClose={close}
        tone="warning"
        title="The device is not connected"
        description="Changes are saved and appear on the Stream Deck when it is connected again."
      />
      <AlertDialog
        open={open === "alert-error"}
        onClose={close}
        tone="error"
        title="The action could not run"
        description="Docker is not running. Start Docker Desktop and press the key again."
      />
      {open === "form" && <ShowcaseForm onClose={close} />}
      <FormDialog
        open={open === "form-long"}
        onClose={close}
        title="Layout details"
        description="A form whose content is taller than the dialog: the body scrolls, the footer stays."
        submitLabel="Save"
        onSubmit={() => wait(500)}
      >
        {Array.from({ length: 4 }, (_, i) => (
          <p key={i} className="text-sm text-muted-foreground">
            {LONG}
          </p>
        ))}
      </FormDialog>
      <SelectionDialog
        open={open === "selection"}
        onClose={close}
        title="Move the page to"
        description="The page and its keys move to the chosen layout."
        options={[
          { id: "main", label: "Main", description: "3 pages" },
          { id: "coding", label: "Coding", description: "2 pages" },
          {
            id: "streaming",
            label: "Streaming",
            description: "Used by no profile",
            disabled: true,
          },
        ]}
        submitLabel="Move"
        onSelect={() => wait(500)}
      />
      <SelectionDialog
        open={open === "selection-long"}
        onClose={close}
        title="Choose a profile"
        options={Array.from({ length: 24 }, (_, i) => ({
          id: `${i}`,
          label: `Profile ${i + 1}`,
          description: i % 3 === 0 ? "Active on Stream Deck MK.2" : undefined,
        }))}
        onSelect={() => undefined}
      />
      <PermissionDialog
        open={open === "permission"}
        onClose={close}
        requester={{ name: "GitHub", detail: "deckpilot.github 0.1.0 · by DeckPilot" }}
        title="Allow GitHub to use these permissions?"
        description="It asks for them to show pull requests and run checks on keys."
        status="2 permissions, 1 needs your explicit consent"
        allowLabel="Allow and install"
        onAllow={() => wait(800)}
      >
        <ul className="grid gap-2 text-sm">
          <li className="flex items-baseline justify-between gap-4">
            <span>Network: api.github.com</span>
            <span className="text-xs text-muted-foreground">Medium</span>
          </li>
          <li className="flex items-baseline justify-between gap-4">
            <span>Credential: GitHub token</span>
            <span className="text-xs text-destructive">High</span>
          </li>
        </ul>
      </PermissionDialog>
      {open === "progress" && <ShowcaseProgress onClose={close} />}
      <ProgressDialog
        open={open === "progress-indeterminate"}
        title="Checking the plugin"
        description="Verifying the signature and the pinned digest."
        value={null}
        onCancel={close}
      />
      {open === "credential" && (
        <CredentialDialog
          icon={KeyRound}
          title="Enter your password"
          description="Authenticate to export your configuration."
          submitLabel="Confirm"
          submit={async (password) => {
            await wait(600)
            if (password === "agent") throw new Error("the DeckPilot agent is not running")
            if (password !== "deckpilot") throw new Error("the password is not correct")
          }}
          onDone={close}
          onCancel={close}
        />
      )}
    </>
  )
}

function Row({
  label,
  description,
  children,
}: {
  label: string
  description: string
  children: React.ReactNode
}) {
  return (
    <SettingsRow label={label} description={description}>
      <div className="flex flex-wrap justify-end gap-2">{children}</div>
    </SettingsRow>
  )
}

/** A native window preview: a bordered box at a fixed width. */
function Frame({ width, children }: { width: number; children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-lg border shadow-sm" style={{ width }}>
      <div className="flex h-7 items-center gap-1.5 border-b bg-muted px-3" aria-hidden>
        {[0, 1, 2].map((i) => (
          <span key={i} className="size-2.5 rounded-full bg-muted-foreground/30" />
        ))}
      </div>
      <div className="[&_[data-modal-root]]:w-full [&_[data-modal-root]]:min-w-0">{children}</div>
    </div>
  )
}

/** Mounted only while open, so every opening starts empty. */
function ShowcaseForm({ onClose }: { onClose: () => void }) {
  const [name, setName] = useState("")
  const [touched, setTouched] = useState(false)
  const invalid = touched && name.trim().length < 3
  return (
    <FormDialog
      open
      onClose={onClose}
      icon={Plus}
      title="New layout"
      description="For Stream Deck MK.2 (15 keys)."
      submitLabel="Create"
      busyLabel="Creating…"
      canSubmit={name.trim().length >= 3}
      dirty={name.length > 0}
      onSubmit={async () => {
        await wait(900)
        if (name.trim().toLowerCase() === "main") throw new Error("A layout named “Main” exists.")
      }}
    >
      <div className="grid gap-2">
        <Label htmlFor="showcase-name">Name</Label>
        <Input
          id="showcase-name"
          autoFocus
          value={name}
          aria-invalid={invalid || undefined}
          aria-describedby={invalid ? "showcase-name-error" : undefined}
          onBlur={() => {
            setTouched(true)
          }}
          onChange={(e) => {
            setName(e.target.value)
          }}
        />
        <FieldError
          id="showcase-name-error"
          message={invalid ? "Use at least 3 characters." : null}
          reserve
        />
      </div>
      <p className="flex items-center gap-2 text-xs text-muted-foreground">
        <FolderGit2 className="size-3.5" aria-hidden /> Try “Main” to see a save error.
      </p>
    </FormDialog>
  )
}

/** Mounted only while open; counts to 100 %, then closes. */
function ShowcaseProgress({ onClose }: { onClose: () => void }) {
  const [value, setValue] = useState(0)
  useEffect(() => {
    const timer = setInterval(() => {
      setValue((v) => {
        if (v >= 100) {
          clearInterval(timer)
          setTimeout(onClose, 400)
          return 100
        }
        return v + 10
      })
    }, 300)
    return () => {
      clearInterval(timer)
    }
  }, [onClose])
  return (
    <ProgressDialog
      open
      title="Installing GitHub"
      description="Downloading and verifying the plugin."
      value={value}
      status={`${value} %`}
    />
  )
}
