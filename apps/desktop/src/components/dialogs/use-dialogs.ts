import { createContext, useContext } from "react"

import type { Dialogs } from "./dialog-service"

export const DialogsContext = createContext<Dialogs | null>(null)

/**
 * Whether the surrounding `DialogProvider` shows one of its own dialogs
 * now (or has some waiting). Plugin dialogs wait behind them
 * (plugins/dialog-host.tsx).
 */
export const ServiceDialogOpenContext = createContext(false)

/** Whether a service dialog is open or waiting (`ServiceDialogOpenContext`). */
export function useServiceDialogOpen(): boolean {
  return useContext(ServiceDialogOpenContext)
}

/** The dialog service of the surrounding `DialogProvider` (dialog-service.tsx). */
export function useDialogs(): Dialogs {
  const value = useContext(DialogsContext)
  if (!value) throw new Error("useDialogs must be used inside DialogProvider")
  return value
}
