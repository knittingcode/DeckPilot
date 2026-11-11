// Records toasts instead of showing them:
//
//   vi.mock("sonner", async () => (await import("@/test/toast")).sonnerModule)
//   import { toast } from "@/test/toast"

import { vi } from "vitest"

export const toast = {
  success: vi.fn(),
  error: vi.fn(),
  warning: vi.fn(),
  info: vi.fn(),
  message: vi.fn(),
}

export function resetToasts() {
  for (const fn of Object.values(toast)) fn.mockReset()
}

export const sonnerModule = { toast, Toaster: () => null }
