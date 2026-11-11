// A typed mock of `@/lib/api` for tests. Every command is a `vi.fn` created
// on first use, so tests only set up what they call:
//
//   vi.mock("@/lib/api", async () => (await import("@/test/api-mock")).apiModule)
//   import { api, resetApi } from "@/test/api-mock"
//   api.getProfile.mockResolvedValue(view)
//
// Unconfigured commands resolve to `undefined`.

import { type Mock, vi } from "vitest"

import type { api as realApi } from "@/lib/api"

type Api = typeof realApi
type Fn = (...args: never[]) => unknown

export type ApiMock = { [K in keyof Api]: Api[K] extends Fn ? Mock<Api[K]> : never }

const commands = new Map<string, Mock>()

function command(name: string): Mock {
  let fn = commands.get(name)
  if (!fn) {
    fn = vi.fn(() => Promise.resolve(undefined))
    commands.set(name, fn)
  }
  return fn
}

export const api = new Proxy({} as ApiMock, {
  get: (_, name) => (typeof name === "string" && name !== "then" ? command(name) : undefined),
})

/** Clears calls and configured results of every command. */
export function resetApi() {
  for (const fn of commands.values()) fn.mockReset()
}

/** Like the real `errorMessage`: errors as the backend reports them. */
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** The module `@/lib/api` is replaced with. */
export const apiModule = { api, errorMessage }
