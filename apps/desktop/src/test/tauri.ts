// Mocks of Tauri modules for tests:
//
//   vi.mock("@tauri-apps/api/event", async () => (await import("@/test/tauri")).eventModule)
//   import { emitBackend } from "@/test/tauri"
//   emitBackend("agent-notification", payload)

import { vi } from "vitest"

type Handler = (event: { payload: unknown }) => void

/** Listeners the app registered for backend events, by event name. */
export const backendListeners = new Map<string, Set<Handler>>()

export const listen = vi.fn((name: string, handler: Handler) => {
  const handlers = backendListeners.get(name) ?? new Set<Handler>()
  handlers.add(handler)
  backendListeners.set(name, handlers)
  return Promise.resolve(() => {
    handlers.delete(handler)
  })
})

/** Delivers a backend event to every registered listener; returns how many got it. */
export function emitBackend(name: string, payload: unknown): number {
  const handlers = [...(backendListeners.get(name) ?? [])]
  for (const handler of handlers) handler({ payload })
  return handlers.length
}

export function resetBackend() {
  backendListeners.clear()
  listen.mockClear()
}

/** The module `@tauri-apps/api/event` is replaced with. */
export const eventModule = { listen, emit: vi.fn(() => Promise.resolve()) }
