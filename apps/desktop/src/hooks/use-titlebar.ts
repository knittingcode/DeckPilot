import { createContext, useContext, useEffect, useId, useSyncExternalStore } from "react"

import { type TitlebarConfig, TitlebarStore } from "@/lib/titlebar"

export const TitlebarContext = createContext<TitlebarStore | null>(null)

/**
 * Contributes to the shared titlebar while the calling component is
 * mounted: the page title, actions, status or progress. Any component of a
 * page may call it, including module pages. Outside a titlebar (native
 * dialog windows) it does nothing.
 */
export function useTitlebar(config: TitlebarConfig) {
  const store = useContext(TitlebarContext)
  const id = useId()
  useEffect(() => {
    store?.set(id, config)
  })
  useEffect(
    () => () => {
      store?.remove(id)
    },
    [store, id],
  )
}

/** The merged configuration; used by the titlebar. */
export function useTitlebarState(store: TitlebarStore) {
  return useSyncExternalStore(store.subscribe, store.getSnapshot)
}
