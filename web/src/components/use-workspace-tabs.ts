import { useEffect, useMemo, useState } from 'react'
import {
  closeWorkspaceTab,
  emptyWorkspaceTabState,
  openWorkspaceTab,
  pruneWorkspaceTabs,
  type WorkspaceTabItem,
} from './workspace-tab-model'

/**
 * Open tabs are view state: the route owns which file is being read, this owns
 * which files are within reach. Nothing here outlives the page.
 */
export function useWorkspaceTabs({
  activeId,
  items,
}: {
  activeId: string | null
  items: WorkspaceTabItem[]
}) {
  const itemById = useMemo(
    () => new Map(items.map((item) => [item.id, item])),
    [items],
  )
  const [state, setState] = useState(() =>
    activeId ? openWorkspaceTab(emptyWorkspaceTabState, activeId, false) : emptyWorkspaceTabState,
  )

  // A file reached without opening a tab — a deep link or browser history entry
  // — becomes the preview tab. Closing never routes to a closed file, so this
  // cannot resurrect one.
  useEffect(() => {
    if (!activeId) return
    setState((current) => openWorkspaceTab(current, activeId, false))
  }, [activeId])

  const openState = useMemo(
    () => pruneWorkspaceTabs(state, new Set(itemById.keys())),
    [itemById, state],
  )
  const tabs = useMemo(
    () =>
      openState.openIds.flatMap((id) => {
        const item = itemById.get(id)
        return item ? [item] : []
      }),
    [itemById, openState.openIds],
  )

  return {
    close(id: string) {
      const result = closeWorkspaceTab(openState, activeId, id)
      setState(result.state)
      return result
    },
    open(id: string, pinned: boolean) {
      setState((current) => openWorkspaceTab(current, id, pinned))
    },
    previewId: openState.previewId,
    tabs,
  }
}
