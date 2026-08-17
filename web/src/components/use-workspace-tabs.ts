import { useEffect, useState } from 'react'
import {
  closeWorkspaceTab,
  emptyWorkspaceTabState,
  openWorkspaceTab,
  pruneWorkspaceTabs,
} from './workspace-tab-model'

/**
 * Open tabs are view state: the route owns which file is being read, this owns
 * which files are within reach. Nothing here outlives the page.
 */
export function useWorkspaceTabs({
  activeId,
}: {
  activeId: string | null
}) {
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

  return {
    close(id: string, availableIds: ReadonlySet<string>) {
      const result = closeWorkspaceTab(
        pruneWorkspaceTabs(state, availableIds),
        activeId,
        id,
      )
      setState(result.state)
      return result
    },
    open(id: string, pinned: boolean) {
      setState((current) => openWorkspaceTab(current, id, pinned))
    },
    state,
  }
}
