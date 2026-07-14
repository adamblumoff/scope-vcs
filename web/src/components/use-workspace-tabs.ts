import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import {
  closeWorkspaceTab,
  normalizeWorkspaceTabIds,
  type WorkspaceTabItem,
} from './workspace-tab-model'

const STORAGE_PREFIX = 'scope-workspace-tabs:'
const EMPTY_IDS: string[] = []
const listenersByKey = new Map<string, Set<() => void>>()
const memoryOnlyKeys = new Set<string>()
const storedSnapshots = new Map<string, { ids: string[]; raw: string | null }>()

export function useWorkspaceTabs({
  activeId,
  items,
  storageKey,
}: {
  activeId: string | null
  items: WorkspaceTabItem[]
  storageKey: string
}) {
  const itemById = useMemo(
    () => new Map(items.map((item) => [item.id, item])),
    [items],
  )
  const availableIds = useMemo(() => new Set(itemById.keys()), [itemById])
  const [suppressedId, setSuppressedId] = useState<string | null>(null)
  const subscribeToStorage = useCallback(
    (listener: () => void) => subscribe(storageKey, listener),
    [storageKey],
  )
  const storedIds = useSyncExternalStore(
    subscribeToStorage,
    () => readStoredIds(storageKey),
    () => EMPTY_IDS,
  )
  const normalizedActiveId = activeId === suppressedId ? null : activeId
  const openIds = useMemo(
    () => normalizeWorkspaceTabIds(
      storedIds.filter((id) => id !== suppressedId),
      availableIds,
      normalizedActiveId,
    ),
    [availableIds, normalizedActiveId, storedIds, suppressedId],
  )

  useEffect(() => {
    if (suppressedId === null || activeId === suppressedId) return
    setSuppressedId(null)
  }, [activeId, suppressedId])
  const tabs = useMemo(
    () =>
      openIds.flatMap((id) => {
        const item = itemById.get(id)
        return item ? [item] : []
      }),
    [itemById, openIds],
  )

  return {
    close(id: string) {
      const result = closeWorkspaceTab(openIds, normalizedActiveId, id)
      if (id === normalizedActiveId) setSuppressedId(id)
      writeStoredIds(storageKey, result.openIds)
      return result
    },
    prepareOpen(id: string) {
      setSuppressedId((current) => (current === id ? null : current))
      const nextIds = normalizeWorkspaceTabIds(
        [...openIds, id],
        availableIds,
        id,
      )
      writeStoredIds(storageKey, nextIds, false)
    },
    tabs,
  }
}

function readStoredIds(storageKey: string) {
  const key = `${STORAGE_PREFIX}${storageKey}`
  if (memoryOnlyKeys.has(key)) {
    return storedSnapshots.get(key)?.ids ?? EMPTY_IDS
  }
  if (typeof sessionStorage === 'undefined') {
    return storedSnapshots.get(key)?.ids ?? EMPTY_IDS
  }
  try {
    const raw = sessionStorage.getItem(key)
    const cached = storedSnapshots.get(key)
    if (cached?.raw === raw) return cached.ids
    const value: unknown = JSON.parse(raw ?? '[]')
    const ids = Array.isArray(value)
      ? value.filter((id): id is string => typeof id === 'string')
      : []
    storedSnapshots.set(key, { ids, raw })
    return ids
  } catch {
    return storedSnapshots.get(key)?.ids ?? EMPTY_IDS
  }
}

function writeStoredIds(storageKey: string, ids: string[], notify = true) {
  const key = `${STORAGE_PREFIX}${storageKey}`
  const raw = JSON.stringify(ids)
  try {
    if (typeof sessionStorage !== 'undefined') sessionStorage.setItem(key, raw)
    memoryOnlyKeys.delete(key)
  } catch {
    memoryOnlyKeys.add(key)
    // Session persistence is optional when browser storage is unavailable.
  }
  storedSnapshots.set(key, { ids, raw })
  if (notify) {
    for (const listener of listenersByKey.get(key) ?? []) listener()
  }
}

function subscribe(storageKey: string, listener: () => void) {
  const key = `${STORAGE_PREFIX}${storageKey}`
  let listeners = listenersByKey.get(key)
  if (!listeners) {
    listeners = new Set()
    listenersByKey.set(key, listeners)
  }
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0) listenersByKey.delete(key)
  }
}
