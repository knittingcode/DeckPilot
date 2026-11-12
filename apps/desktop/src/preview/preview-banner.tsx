import { useState } from "react"

// Integration preview only (preview/integration branch, never merged): a
// permanent marker so the preview cannot be mistaken for a release. Filled
// from VITE_DECKPILOT_PREVIEW, set by tools/preview/start.sh.
interface PreviewInfo {
  base: string
  included: number[]
  excluded: { pr: number; reason: string }[]
  built: string
  version: string
}

function previewInfo(): PreviewInfo | null {
  const raw = import.meta.env.VITE_DECKPILOT_PREVIEW as string | undefined
  if (!raw) return null
  try {
    return JSON.parse(raw) as PreviewInfo
  } catch {
    return null
  }
}

export function PreviewBanner() {
  const info = previewInfo()
  const [open, setOpen] = useState(false)
  if (!info) return null
  return (
    <div
      style={{ zIndex: 2147483647 }}
      className="fixed bottom-2 left-1/2 max-w-[90vw] -translate-x-1/2 rounded-md border border-amber-500 bg-amber-400 px-3 py-1 text-xs text-black shadow-lg"
    >
      <button type="button" className="font-semibold" onClick={() => setOpen((o) => !o)}>
        Integration preview · {info.version} · main {info.base} · {info.included.length} PRs
      </button>
      {open && (
        <div className="mt-1 space-y-0.5 text-left">
          <div>Built {info.built}</div>
          <div>Included: {info.included.map((n) => `#${n}`).join(" ")}</div>
          {info.excluded.map((e) => (
            <div key={e.pr}>
              Excluded #{e.pr}: {e.reason}
            </div>
          ))}
          <div>Not a release. Isolated data; updates disabled.</div>
        </div>
      )}
    </div>
  )
}
