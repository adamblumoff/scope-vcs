export type WorkspaceTabItem = {
  id: string
  label: string
  title?: string
}

/**
 * Open tabs plus the single preview slot. A previewed tab is replaced by the
 * next previewed file instead of accumulating, matching editor conventions.
 */
export type WorkspaceTabState = {
  openIds: string[]
  previewId: string | null
}

export const emptyWorkspaceTabState: WorkspaceTabState = {
  openIds: [],
  previewId: null,
}

export function workspaceTabDomIds(tabSetId: string, tabId: string) {
  return {
    panelId: workspaceTabPanelId(tabSetId),
    tabId: `${tabSetId}-tab-${encodeURIComponent(tabId)}`,
  }
}

export function workspaceTabPanelId(tabSetId: string) {
  return `${tabSetId}-panel`
}

export function openWorkspaceTab(
  state: WorkspaceTabState,
  id: string,
  pinned: boolean,
): WorkspaceTabState {
  if (state.openIds.includes(id)) {
    if (!pinned || state.previewId !== id) return state
    return { openIds: state.openIds, previewId: null }
  }

  const previewIndex = state.previewId
    ? state.openIds.indexOf(state.previewId)
    : -1
  const openIds = [...state.openIds]
  if (previewIndex === -1) openIds.push(id)
  else openIds[previewIndex] = id

  return { openIds, previewId: pinned ? null : id }
}

export function closeWorkspaceTab(
  state: WorkspaceTabState,
  activeId: string | null,
  closingId: string,
) {
  const closingIndex = state.openIds.indexOf(closingId)
  if (closingIndex === -1) {
    return { activeId, focusId: activeId, state }
  }

  const openIds = state.openIds.filter((id) => id !== closingId)
  const neighborId = openIds[Math.min(closingIndex, openIds.length - 1)] ?? null
  return {
    activeId: activeId === closingId ? neighborId : activeId,
    focusId: neighborId,
    state: {
      openIds,
      previewId: state.previewId === closingId ? null : state.previewId,
    },
  }
}

/** Drops tabs whose file left the projection. Never re-adds anything. */
export function pruneWorkspaceTabs(
  state: WorkspaceTabState,
  availableIds: ReadonlySet<string>,
): WorkspaceTabState {
  const openIds = state.openIds.filter((id) => availableIds.has(id))
  if (openIds.length === state.openIds.length) return state
  return {
    openIds,
    previewId:
      state.previewId && openIds.includes(state.previewId)
        ? state.previewId
        : null,
  }
}

/**
 * Labels tabs by filename, extending colliding labels one parent segment at a
 * time until they are unique. Full paths never fit inside a tab.
 */
export function workspaceTabVisibleLabels(tabs: readonly WorkspaceTabItem[]) {
  const collisions = new Map<string, WorkspaceTabItem[]>()
  for (const tab of tabs) {
    const group = collisions.get(tab.label)
    if (group) group.push(tab)
    else collisions.set(tab.label, [tab])
  }

  const labels = new Map<string, string>()
  for (const group of collisions.values()) {
    if (group.length === 1) {
      labels.set(group[0].id, group[0].label)
      continue
    }
    for (const tab of group) {
      labels.set(tab.id, uniqueSuffix(tab, group))
    }
  }
  return labels
}

function uniqueSuffix(tab: WorkspaceTabItem, group: WorkspaceTabItem[]) {
  const segments = pathSegments(tab)
  for (let depth = 2; depth < segments.length; depth += 1) {
    const candidate = suffix(segments, depth)
    const shared = group.some(
      (other) => other !== tab && suffix(pathSegments(other), depth) === candidate,
    )
    if (!shared) return candidate
  }
  return suffix(segments, segments.length)
}

function pathSegments(tab: WorkspaceTabItem) {
  return (tab.title ?? tab.id).split('/').filter(Boolean)
}

function suffix(segments: string[], depth: number) {
  return segments.slice(-depth).join('/')
}
