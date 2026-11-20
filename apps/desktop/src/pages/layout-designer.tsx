import { Info } from "lucide-react"
import { type ReactNode, useEffect } from "react"

import { EmptyState, ErrorNotice, Loading } from "@/components/page"
import { RequireProfile } from "@/components/require-profile"
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import { useAsync } from "@/hooks/use-async"
import { useTranslation } from "@/i18n"
import { LayoutEditor } from "@/layout-editor/editor"
import { DeviceLayoutHint, SharedLayoutNote } from "@/layout-editor/layout-chooser"
import { defaultLayoutOf, resolveLayout } from "@/layout-editor/layout-selection"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import { useFocusTarget } from "@/lib/focus"
import { shownLayout } from "@/lib/layout-navigation"
import type { DeviceModel, Platform } from "@/lib/types"
import { LEVELS, PLATFORM_LABELS, currentPlatform, verificationOn } from "@/lib/verification"
import { useEditorSelection } from "@/shell/editor-selection-context"
import { selectedDevice } from "@/shell/device-options"
import { useOpenLayout } from "@/shell/open-layout"

export function LayoutDesignerPage() {
  // No title in the toolbar: the device, profile and layout selectors
  // there say what is edited. The page keeps a heading for assistive technology.
  const { t } = useTranslation()
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <h1 className="sr-only">{t("navigation:pages.designer")}</h1>
      <RequireProfile>{(profileId) => <Designer profileId={profileId} />}</RequireProfile>
    </div>
  )
}

/**
 * The device editor. Profile, device and layout come from the toolbar (the
 * shell's editor selection); without a chosen layout, the profile's default
 * for the model.
 */
function Designer({ profileId }: { profileId: string }) {
  const { t } = useTranslation()
  const { revision, agent } = useAppContext()
  const selection = useEditorSelection()
  const view = useAsync(() => api.getProfile(profileId), `${profileId}:${revision}`)
  const shared = useAsync(() => api.listSharedLayouts(), `shared-layouts:${revision}`)
  const { open: openLayout, dialog: openDialog } = useOpenLayout()
  const models = selection.models
  // Opened from another page (e.g. the references of an action): that layout.
  const focus = useFocusTarget("layout")
  const { showLayout, adoptModel, setDirty } = selection
  useEffect(() => {
    if (focus) showLayout(focus.layout)
  }, [focus, showLayout])
  const ready = view.state.status === "ready" && models.status === "ready"
  const profile = view.state.status === "ready" ? view.state.data.profile : null
  const catalog = models.status === "ready" ? models.data : []
  const connected = (agent?.status?.devices ?? [])
    .filter((d) => d.state === "connected")
    .map((d) => d.descriptor?.modelId ?? d.id.model)
  const focused = profile?.layouts.find((l) => l.id === selection.layoutId)
  const model = profile
    ? (catalog.find((m) => m.id === focused?.model) ??
      catalog.find((m) => m.id === selection.modelId) ??
      // Nothing chosen yet: a connected device first.
      catalog.find((m) => connected.includes(m.id)) ??
      catalog.find((m) => profile.layouts.some((l) => l.model === m.id)) ??
      catalog[0])
    : undefined
  // The toolbar shows the model the editor settled on.
  useEffect(() => {
    if (model && model.id !== selection.modelId) adoptModel(model.id)
  }, [model, selection.modelId, adoptModel])
  // Nothing unsaved once the editor is gone.
  useEffect(
    () => () => {
      setDirty(false)
    },
    [setDirty],
  )

  if (!ready) {
    if (view.state.status === "error")
      return (
        <Padded>
          <ErrorNotice error={view.state.error} />
        </Padded>
      )
    if (models.status === "error")
      return (
        <Padded>
          <ErrorNotice error={models.error} />
        </Padded>
      )
    return (
      <Padded>
        <Loading />
      </Padded>
    )
  }
  if (!model || !profile || view.state.status !== "ready") {
    return (
      <Padded>
        <EmptyState title={t("editor:designer.noModels")} />
      </Padded>
    )
  }
  const layouts = profile.layouts.filter((l) => l.model === model.id)
  // The device the editor is for, and the layout it shows (of the active profile).
  const device = selectedDevice(agent?.status?.devices ?? [], model.id, selection.device)
  const shownId = shownLayout(device)
  // Nothing chosen: the layout the device shows, when it runs this profile
  // (the editor follows the device), else the profile's default.
  const activeForModel = defaultLayoutOf(
    layouts,
    profile.activeLayouts?.[model.id],
    shownId,
    agent?.status?.activeProfile?.id === profileId,
  )
  const layout = resolveLayout(layouts, selection.layoutId, activeForModel)
  const sharedLayouts =
    shared.state.status === "ready" ? shared.state.data.filter((l) => l.model === model.id) : null
  const shownName =
    device && "name" in device.layout
      ? device.layout.name
      : (sharedLayouts?.find((l) => l.id === shownId)?.name ?? null)
  const openShown = () => {
    if (!shownId) return
    const known = sharedLayouts?.find((l) => l.id === shownId)
    if (known) openLayout(known)
    else if (layouts.some((l) => l.id === shownId)) selection.selectLayout(shownId)
  }

  // The layout's own notes; choosing and managing layouts is in the
  // toolbar's layout selector (layout-editor/layout-management.tsx).
  const notices = (
    <>
      <SharedLayoutNote layout={sharedLayouts?.find((l) => l.id === layout?.id)} />
      {shownId && layout && shownId !== layout.id && (
        <DeviceLayoutHint name={shownName ?? "another layout"} onOpen={openShown} />
      )}
      {openDialog}
      <VerificationNotice model={model} />
    </>
  )

  return layout ? (
    <LayoutEditor
      // A renamed layout is a fresh editor (renaming asks to discard edits first).
      key={`${profileId}:${layout.id}:${layout.name}`}
      profileId={profileId}
      view={view.state.data}
      model={model}
      saved={layout}
      notices={notices}
      device={device}
      onDirtyChange={setDirty}
      onSaved={() => {
        setDirty(false)
        view.reload()
      }}
    />
  ) : (
    <Padded>
      {notices}
      <EmptyState title={t("editor:designer.noLayout", { model: model.name })}>
        {t("editor:designer.noLayoutHint", {
          newLayout: t("layouts:menu.new"),
          layouts: t("navigation:pages.layouts"),
        })}
      </EmptyState>
    </Padded>
  )
}

function Padded({ children }: { children: ReactNode }) {
  return <div className="grid content-start gap-4 p-4">{children}</div>
}

/** Shown unless the model was tested on a physical device on this platform. */
export function VerificationNotice({
  model,
  platform = currentPlatform(),
}: {
  model: DeviceModel
  platform?: Platform
}) {
  const { t } = useTranslation()
  const entry = verificationOn(model, platform)
  if (entry.level === "physicallyVerified") return null
  return (
    <Alert>
      <Info />
      <AlertTitle>
        {t("devices:verification.onPlatform", {
          level: t(LEVELS[entry.level].label),
          platform: PLATFORM_LABELS[platform],
        })}
      </AlertTitle>
      <AlertDescription>{entry.evidence || t(LEVELS[entry.level].detail)}</AlertDescription>
    </Alert>
  )
}
