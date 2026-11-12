// Bounded undo/redo history shared by the layout editor and the Workflow
// Builder.

/** Bounded undo/redo history of committed values. */
export interface History<T> {
  past: T[]
  present: T
  future: T[]
  /** Consecutive commits with the same key (typing) form one undo step. */
  group: string | null
}

export const HISTORY_LIMIT = 100

const same = <T>(a: T, b: T) => a === b || JSON.stringify(a) === JSON.stringify(b)

export function history<T>(value: T): History<T> {
  return { past: [], present: value, future: [], group: null }
}

export function commit<T>(
  state: History<T>,
  value: T,
  group: string | null = null,
  limit = HISTORY_LIMIT,
): History<T> {
  if (same(value, state.present)) return state
  if (group !== null && group === state.group) {
    return { ...state, present: value, future: [] }
  }
  return {
    past: [...state.past, state.present].slice(-limit),
    present: value,
    future: [],
    group,
  }
}

/** Replaces the present without an undo step (e.g. while dragging). */
export function replace<T>(state: History<T>, value: T): History<T> {
  return { ...state, present: value }
}

/**
 * Ends a series of replacements (a drag) that started at `before`: one
 * undo step returns to it.
 */
export function settle<T>(state: History<T>, before: T, limit = HISTORY_LIMIT): History<T> {
  if (same(before, state.present)) return state
  return {
    past: [...state.past, before].slice(-limit),
    present: state.present,
    future: [],
    group: null,
  }
}

export function undo<T>(state: History<T>): History<T> {
  const previous = state.past.at(-1)
  if (previous === undefined) return state
  return {
    past: state.past.slice(0, -1),
    present: previous,
    future: [state.present, ...state.future],
    group: null,
  }
}

export function redo<T>(state: History<T>): History<T> {
  const [next, ...future] = state.future
  if (next === undefined) return state
  return { past: [...state.past, state.present], present: next, future, group: null }
}

export function canUndo(state: History<unknown>): boolean {
  return state.past.length > 0
}

export function canRedo(state: History<unknown>): boolean {
  return state.future.length > 0
}
