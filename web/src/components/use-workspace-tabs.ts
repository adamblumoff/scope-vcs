import { useMemo, useSyncExternalStore } from 'react'
import {
  closeWorkspaceTab,
  normalizeWorkspaceTabIds,
  type WorkspaceTabItem,
} from './workspace-tab-model'

const STORAGE_PREFIX = 'scope-workspace-tabs:'
const EMPTY_IDS: string[] = []
const listeners = new Set<() => void>()
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
  const storedIds = useSyncExternalStore(
    subscribe,
    () => readStoredIds(storageKey),
    () => EMPTY_IDS,
  )
  const openIds = useMemo(
    () => normalizeWorkspaceTabIds(storedIds, availableIds, activeId),
    [activeId, availableIds, storedIds],
  )
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
      const result = closeWorkspaceTab(openIds, activeId, id)
      writeStoredIds(storageKey, result.openIds)
      return result
    },
    open(id: string) {
      const nextIds = normalizeWorkspaceTabIds(
        [...openIds, id],
        availableIds,
        id,
      )
      writeStoredIds(storageKey, nextIds)
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

function writeStoredIds(storageKey: string, ids: string[]) {
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
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}
