import { createContext, useContext, useEffect, useSyncExternalStore } from "react"

import { NotificationStore, OperationStore, type StatusItem, StatusItemStore } from "@/lib/status"

export interface StatusStores {
  items: StatusItemStore
  operations: OperationStore
  notifications: NotificationStore
}

export function createStatusStores(): StatusStores {
  return {
    items: new StatusItemStore(),
    operations: new OperationStore(),
    notifications: new NotificationStore(),
  }
}

export const StatusContext = createContext<StatusStores | null>(null)

/** The status stores; `null` outside the main window (dialog windows). */
export function useStatus(): StatusStores | null {
  return useContext(StatusContext)
}

/**
 * Shows `item` in the status bar while the calling component is mounted and
 * `item` is not `null` (its visibility condition). Modules call this from
 * the components they register in `FrontendModule.statusItems`, which only
 * mount while the module is enabled.
 */
export function useStatusItem(item: StatusItem | null) {
  const stores = useStatus()
  const id = item?.id
  useEffect(() => {
    if (item) stores?.items.set(item)
  })
  useEffect(() => {
    if (!id) return
    return () => {
      stores?.items.remove(id)
    }
  }, [stores, id])
}

export function useSnapshot<T>(store: {
  subscribe: (listener: () => void) => () => void
  getSnapshot: () => T
}) {
  return useSyncExternalStore(store.subscribe, store.getSnapshot)
}
