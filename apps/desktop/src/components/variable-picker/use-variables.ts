import { listen } from "@tauri-apps/api/event"
import { type KeyboardEvent, useEffect, useRef, useState } from "react"

import { api, errorMessage } from "@/lib/api"
import type {
  CatalogEntry,
  CatalogFilter,
  ParameterKind,
  PreviewItem,
  ReferenceStatus,
  ResolutionContext,
} from "@/lib/types"
import {
  SYSTEM_ENTRIES,
  VARIABLES_EVENT,
  type VariableContextEvent,
  eventAffects,
  groupEntries,
  isStepReference,
  matchesFilter,
} from "@/lib/variables"

import { withoutDeprecated } from "@/lib/variable-catalog"

import type { VariableScope } from "./scope"

/** Delay between the last key typed into a search and the request. */
export const SEARCH_DELAY_MS = 150

export function useDebounced<T>(value: T, delay: number): T {
  const [settled, setSettled] = useState(value)
  useEffect(() => {
    const timer = setTimeout(() => {
      setSettled(value)
    }, delay)
    return () => {
      clearTimeout(timer)
    }
  }, [value, delay])
  return settled
}

/**
 * A counter that goes up whenever the agent reports a change that can
 * affect references in `context` (project, profile, shared variables,
 * resource bindings, device, the run). No polling.
 */
export function useVariableRefresh(context: ResolutionContext, enabled: boolean): number {
  const [tick, setTick] = useState(0)
  const contextKey = JSON.stringify(context)
  useEffect(() => {
    if (!enabled) return
    const current = JSON.parse(contextKey) as ResolutionContext
    let stop: Promise<() => void> | undefined
    try {
      stop = listen<VariableContextEvent>(VARIABLES_EVENT, ({ payload }) => {
        if (eventAffects(payload, current)) setTick((t) => t + 1)
      })
    } catch {
      // Outside Tauri there are no agent events.
    }
    return () => {
      void stop?.then(
        (unlisten) => {
          unlisten()
        },
        () => undefined,
      )
    }
  }, [contextKey, enabled])
  return tick
}

export interface CatalogState {
  entries: CatalogEntry[]
  loading: boolean
  /** Why the agent could not list variables; only local metadata is shown. */
  unavailable?: string
}

/**
 * References that can be inserted into a field of `kind` in the scope,
 * found by `query` (debounced). The agent lists them; outputs of the graph
 * being edited are added from the scope. A response to an older search is
 * dropped.
 */
export function useVariableCatalog(
  scope: VariableScope,
  query: string,
  kind: ParameterKind | undefined,
  enabled = true,
): CatalogState {
  const debounced = useDebounced(query.trim(), SEARCH_DELAY_MS)
  const live = enabled && scope.live
  const tick = useVariableRefresh(scope.context, live)
  const filter: CatalogFilter = { query: debounced, compatibleWith: kind, includeUnavailable: true }
  const contextKey = JSON.stringify(scope.context)
  const filterKey = JSON.stringify(filter)
  const key = `${contextKey}|${filterKey}|${tick}`
  const [result, setResult] = useState<{
    key: string
    entries: CatalogEntry[]
    unavailable?: string
  } | null>(null)
  const generation = useRef(0)

  useEffect(() => {
    if (!live) return
    const current = ++generation.current
    const context = JSON.parse(contextKey) as ResolutionContext
    const request = JSON.parse(filterKey) as CatalogFilter
    const requestKey = `${contextKey}|${filterKey}|${tick}`
    api.listVariables(context, request).then(
      (entries) => {
        if (current === generation.current) setResult({ key: requestKey, entries })
      },
      (error: unknown) => {
        if (current !== generation.current) return
        setResult({
          key: requestKey,
          entries: SYSTEM_ENTRIES.filter((entry) => matchesFilter(entry, request)),
          unavailable: errorMessage(error),
        })
      },
    )
  }, [live, contextKey, filterKey, tick])

  const local = scope.stepOutputs.filter((entry) => matchesFilter(entry, filter))
  const remote = result?.entries ?? []
  const known = new Set(remote.map((e) => e.reference))
  return {
    entries: [...remote, ...local.filter((e) => !known.has(e.reference))],
    loading: live && result?.key !== key,
    unavailable: result?.unavailable,
  }
}

export interface PreviewState {
  /** The resolver's answer for `reference`; `undefined` while unknown. */
  status: (reference: string) => ReferenceStatus | undefined
  loading: boolean
  /** Why previews failed as a whole. */
  error?: string
}

const AT_RUNTIME: ReferenceStatus = { status: "unavailableUntilRuntime" }

/**
 * What the agent's resolver reports for `items` in the scope's context,
 * refreshed on agent events. Step outputs have no value before a run and
 * are not asked for. Obsolete responses are dropped (generation counter),
 * so a slow answer for an earlier selection never replaces a newer one.
 */
export function useReferencePreviews(
  scope: VariableScope,
  items: PreviewItem[],
  enabled = true,
): PreviewState {
  const live = enabled && scope.live
  const tick = useVariableRefresh(scope.context, live)
  const remote = items.filter((item) => item.reference.trim() && !isStepReference(item.reference))
  const contextKey = JSON.stringify(scope.context)
  const itemsKey = JSON.stringify(remote)
  const key = `${contextKey}|${itemsKey}|${tick}`
  const [result, setResult] = useState<{
    key: string
    previews: Map<string, ReferenceStatus>
    error?: string
  } | null>(null)
  const generation = useRef(0)

  useEffect(() => {
    if (!live) return
    const current = ++generation.current
    const request = JSON.parse(itemsKey) as PreviewItem[]
    if (request.length === 0) return
    const context = JSON.parse(contextKey) as ResolutionContext
    const requestKey = `${contextKey}|${itemsKey}|${tick}`
    api.previewReferences(context, request).then(
      (previews) => {
        if (current !== generation.current) return
        setResult({
          key: requestKey,
          previews: new Map(previews.map((p) => [p.reference, p.status])),
        })
      },
      (error: unknown) => {
        if (current === generation.current) {
          setResult({ key: requestKey, previews: new Map(), error: errorMessage(error) })
        }
      },
    )
  }, [live, contextKey, itemsKey, tick])

  // While a refresh is in flight the previous answers stay visible.
  const previews = result?.previews
  return {
    status: (reference) =>
      isStepReference(reference) ? AT_RUNTIME : previews?.get(reference.trim()),
    loading: live && remote.length > 0 && result?.key !== key,
    error: result?.key === key ? result.error : undefined,
  }
}

export interface BrowserState {
  catalog: CatalogState
  groups: ReturnType<typeof groupEntries>
  /** Entries in the order they are shown. */
  ordered: CatalogEntry[]
  active: CatalogEntry | undefined
  setActive: (reference: string) => void
  /**
   * Moves the selection or picks the active entry; `true` when the key was
   * handled.
   */
  handleKey: (event: KeyboardEvent, pick: (entry: CatalogEntry) => void) => boolean
}

/** The element id of option `index` of the list `listId`. */
export const optionId = (listId: string, index: number) => `${listId}-option-${index}`

/** `aria-activedescendant` for a field that owns the list. */
export function activeDescendant(listId: string, browser: BrowserState): string | undefined {
  return browser.active ? optionId(listId, browser.ordered.indexOf(browser.active)) : undefined
}

/** A searchable, grouped list of references with one active entry. */
export function useVariableBrowser(
  scope: VariableScope,
  query: string,
  kind: ParameterKind | undefined,
  enabled = true,
): BrowserState {
  const listed = useVariableCatalog(scope, query, kind, enabled)
  // The picker offers canonical ids only, when the agent lists them
  // (protocol 9); legacy ids keep working but are not inserted any more.
  const catalog = { ...listed, entries: withoutDeprecated(listed.entries) }
  const groups = groupEntries(catalog.entries)
  const ordered = groups.flatMap((g) => g.entries)
  const [activeReference, setActive] = useState<string | null>(null)
  const index = Math.max(
    0,
    ordered.findIndex((e) => e.reference === activeReference),
  )
  const active = ordered[index]
  const select = (next: number) => {
    const entry = ordered[Math.min(ordered.length - 1, Math.max(0, next))]
    if (entry) setActive(entry.reference)
  }
  return {
    catalog,
    groups,
    ordered,
    active,
    setActive,
    handleKey: (event, pick) => {
      switch (event.key) {
        case "ArrowDown":
          select(index + 1)
          break
        case "ArrowUp":
          select(index - 1)
          break
        case "Home":
          select(0)
          break
        case "End":
          select(ordered.length - 1)
          break
        case "Enter":
          if (!active) return false
          pick(active)
          break
        default:
          return false
      }
      event.preventDefault()
      return true
    },
  }
}
