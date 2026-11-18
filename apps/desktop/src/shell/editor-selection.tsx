import { useCallback, useMemo, useState } from "react"
import { toast } from "sonner"

import { useAsync } from "@/hooks/use-async"
import { api } from "@/lib/api"
import { useAppContext } from "@/lib/app-context"
import type { DeviceId } from "@/lib/types"

import {
  type DirtyFlag,
  type EditorSelection,
  EditorSelectionContext,
  UNSAVED_WARNING,
} from "./editor-selection-context"

const MODEL_KEY = "deckpilot.editor.model"

function storedModel(): string | null {
  try {
    return localStorage.getItem(MODEL_KEY)
  } catch {
    return null
  }
}

function storeModel(id: string) {
  try {
    localStorage.setItem(MODEL_KEY, id)
  } catch {
    // Remembered for this session only.
  }
}

export function EditorSelectionProvider({
  dirtyRef,
  children,
}: {
  dirtyRef: DirtyFlag
  children: React.ReactNode
}) {
  const { setEditingProfileId, editingProfileId } = useAppContext()
  const { state: models } = useAsync(() => api.listDeviceModels(), "models")
  const [modelId, setModelId] = useState<string | null>(storedModel)
  const [device, setDevice] = useState<DeviceId | null>(null)
  const [layoutId, setLayoutId] = useState<string | null>(null)

  const guard = useCallback(() => {
    if (!dirtyRef.current) return true
    toast.warning(UNSAVED_WARNING)
    return false
  }, [dirtyRef])

  const chooseModel = useCallback((model: string) => {
    setModelId(model)
    storeModel(model)
  }, [])

  const selectDevice = useCallback(
    (model: string, chosen: DeviceId | null = null) => {
      if (!guard()) return false
      chooseModel(model)
      setDevice(chosen)
      setLayoutId(null)
      return true
    },
    [guard, chooseModel],
  )

  const selectProfile = useCallback(
    (id: string) => {
      if (id === editingProfileId) return true
      if (!guard()) return false
      setLayoutId(null)
      setEditingProfileId(id)
      return true
    },
    [guard, editingProfileId, setEditingProfileId],
  )

  const selectLayout = useCallback(
    (id: string | null) => {
      if (!guard()) return false
      setLayoutId(id)
      return true
    },
    [guard],
  )

  const showLayout = useCallback(
    (layout: string, model?: string | null) => {
      setLayoutId(layout)
      if (model) chooseModel(model)
    },
    [chooseModel],
  )

  const adoptModel = useCallback((model: string) => {
    setModelId(model)
    storeModel(model)
  }, [])

  const setDirty = useCallback(
    (value: boolean) => {
      dirtyRef.current = value
    },
    [dirtyRef],
  )

  const isDirty = useCallback(() => dirtyRef.current, [dirtyRef])

  const value = useMemo<EditorSelection>(
    () => ({
      models,
      modelId,
      device,
      layoutId,
      selectDevice,
      selectProfile,
      selectLayout,
      showLayout,
      adoptModel,
      setDirty,
      guard,
      isDirty,
    }),
    [
      models,
      modelId,
      device,
      layoutId,
      selectDevice,
      selectProfile,
      selectLayout,
      showLayout,
      adoptModel,
      setDirty,
      guard,
      isDirty,
    ],
  )
  return <EditorSelectionContext value={value}>{children}</EditorSelectionContext>
}
