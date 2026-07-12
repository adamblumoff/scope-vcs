export type WorkspaceTabItem = {
  id: string
  label: string
  title?: string
}

export function workspaceTabDomIds(tabSetId: string, tabId: string) {
  const encodedTabId = encodeURIComponent(tabId)
  return {
    panelId: workspaceTabPanelId(tabSetId),
    tabId: `${tabSetId}-tab-${encodedTabId}`,
  }
}

export function workspaceTabPanelId(tabSetId: string) {
  return `${tabSetId}-panel`
}

export function normalizeWorkspaceTabIds(
  openIds: readonly string[],
  availableIds: ReadonlySet<string>,
  activeId: string | null,
) {
  const seen = new Set<string>()
  const normalized: string[] = []

  for (const id of openIds) {
    if (availableIds.has(id) && !seen.has(id)) {
      seen.add(id)
      normalized.push(id)
    }
  }

  if (activeId && availableIds.has(activeId) && !seen.has(activeId)) {
    normalized.push(activeId)
  }

  return normalized
}

export function closeWorkspaceTab(
  openIds: readonly string[],
  activeId: string | null,
  closingId: string,
) {
  const closingIndex = openIds.indexOf(closingId)
  if (closingIndex === -1) {
    return { activeId, focusId: activeId, openIds: [...openIds] }
  }

  const nextOpenIds = openIds.filter((id) => id !== closingId)
  const focusId =
    nextOpenIds[Math.min(closingIndex, nextOpenIds.length - 1)] ?? null
  if (activeId !== closingId) {
    return { activeId, focusId, openIds: nextOpenIds }
  }

  return {
    activeId: focusId,
    focusId,
    openIds: nextOpenIds,
  }
}

export function workspaceTabVisibleLabels(tabs: readonly WorkspaceTabItem[]) {
  const counts = new Map<string, number>()
  for (const tab of tabs) {
    counts.set(tab.label, (counts.get(tab.label) ?? 0) + 1)
  }

  return new Map(
    tabs.map((tab) => [
      tab.id,
      counts.get(tab.label) === 1 ? tab.label : (tab.title ?? tab.label),
    ]),
  )
}
