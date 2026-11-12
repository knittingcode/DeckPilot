import { useCallback, useEffect, useState } from "react"
import { toast } from "sonner"

import { api, errorMessage } from "@/lib/api"
import type { IconInfo } from "@/lib/types"

import { iconFileProblem, iconName, iconSizeProblem } from "./appearance"

/** A backend message as the end of a sentence. */
function sentence(message: string): string {
  const text = message.trim()
  return /[.!?]$/.test(text) ? text : `${text}.`
}

/** The file's content as base64, without the `data:` prefix. */
function readBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = typeof reader.result === "string" ? reader.result : ""
      resolve(result.slice(result.indexOf(",") + 1))
    }
    reader.onerror = () => {
      reject(new Error("The file could not be read."))
    }
    reader.readAsDataURL(file)
  })
}

/** Pixel size of an image file, when the browser can tell without uploading it. */
async function imageSize(file: File): Promise<{ width: number; height: number } | null> {
  if (typeof createImageBitmap !== "function") return null
  try {
    const bitmap = await createImageBitmap(file)
    const size = { width: bitmap.width, height: bitmap.height }
    bitmap.close()
    return size
  } catch {
    // Not decodable here; the backend decides.
    return null
  }
}

/** The profile's icon library: list, import with checks, delete. */
export function useIcons(profileId: string) {
  const [icons, setIcons] = useState<IconInfo[]>([])
  const [loading, setLoading] = useState(true)
  const [busy, setBusy] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)

  const reload = useCallback(() => {
    api.listIcons(profileId).then(
      (list) => {
        setIcons(list)
        setLoading(false)
      },
      (e: unknown) => {
        setLoading(false)
        setProblem(`The icons could not be loaded: ${sentence(errorMessage(e))}`)
      },
    )
  }, [profileId])

  useEffect(() => {
    reload()
  }, [reload])

  /** Imports a file; resolves to the new icon, or `null` when refused. */
  const importFile = async (file: File): Promise<IconInfo | null> => {
    const local = iconFileProblem(file, icons.length)
    if (local) {
      setProblem(local)
      return null
    }
    setBusy(true)
    setProblem(null)
    try {
      const size = await imageSize(file)
      const tooLarge = size && iconSizeProblem(size.width, size.height)
      if (tooLarge) {
        setProblem(tooLarge)
        return null
      }
      const icon = await api.importIcon(profileId, iconName(file.name), await readBase64(file))
      setIcons((list) => [...list.filter((i) => i.id !== icon.id), icon])
      toast.success(`Imported “${icon.name}”`)
      return icon
    } catch (e) {
      setProblem(`“${file.name}” was not imported: ${sentence(errorMessage(e))}`)
      return null
    } finally {
      setBusy(false)
    }
  }

  const remove = async (icon: IconInfo) => {
    setBusy(true)
    setProblem(null)
    try {
      await api.deleteIcon(profileId, icon.id)
      setIcons((list) => list.filter((i) => i.id !== icon.id))
      toast.success(`Deleted “${icon.name}”`)
    } catch (e) {
      setProblem(`“${icon.name}” was not deleted: ${sentence(errorMessage(e))}`)
    } finally {
      setBusy(false)
    }
  }

  return { icons, loading, busy, problem, importFile, remove, reload }
}
