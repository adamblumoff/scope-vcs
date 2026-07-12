import { cn } from '@/lib/utils'
import { FileCode2, X } from 'lucide-react'
import { useRef } from 'react'
import {
  workspaceTabVisibleLabels,
  type WorkspaceTabItem,
} from './workspace-tab-model'

export function WorkspaceTabStrip({
  activeId,
  ariaLabel,
  onActivate,
  onClose,
  onEmptyFocus,
  tabs,
}: {
  activeId: string | null
  ariaLabel: string
  onActivate: (id: string) => void
  onClose: (id: string) => string | null
  onEmptyFocus: () => void
  tabs: WorkspaceTabItem[]
}) {
  const tabRefs = useRef<Map<string, HTMLButtonElement> | null>(null)
  if (tabRefs.current === null) tabRefs.current = new Map()

  if (tabs.length === 0) return null
  const tabStopId = tabs.some((tab) => tab.id === activeId)
    ? activeId
    : tabs[0].id
  const visibleLabels = workspaceTabVisibleLabels(tabs)

  function moveFocus(event: React.KeyboardEvent, id: string) {
    const currentIndex = tabs.findIndex((tab) => tab.id === id)
    let nextIndex: number | null = null
    if (event.key === 'ArrowLeft') {
      nextIndex = (currentIndex - 1 + tabs.length) % tabs.length
    } else if (event.key === 'ArrowRight') {
      nextIndex = (currentIndex + 1) % tabs.length
    } else if (event.key === 'Home') {
      nextIndex = 0
    } else if (event.key === 'End') {
      nextIndex = tabs.length - 1
    }
    if (nextIndex === null) return
    event.preventDefault()
    tabRefs.current?.get(tabs[nextIndex].id)?.focus()
  }

  function closeTab(id: string) {
    const focusId = onClose(id)
    requestAnimationFrame(() => {
      if (focusId) tabRefs.current?.get(focusId)?.focus()
      else onEmptyFocus()
    })
  }

  return (
    <div
      aria-label={ariaLabel}
      className="scrollbar-none flex min-h-10 min-w-0 overflow-x-auto border-b border-border bg-[var(--workspace-tabs)]"
      role="tablist"
    >
      {tabs.map((tab) => {
        const active = tab.id === activeId
        const accessibleLabel = tab.title ?? tab.label
        const visibleLabel = visibleLabels.get(tab.id) ?? tab.label
        return (
          <div
            className={cn(
              'group/tab relative flex min-w-[132px] max-w-[240px] shrink-0 items-center border-r border-border text-muted-foreground transition-[background-color,color,box-shadow] duration-150',
              active &&
                'bg-card text-foreground shadow-[inset_0_2px_0_0_var(--platinum-bright)]',
            )}
            key={tab.id}
          >
            <button
              aria-label={accessibleLabel}
              aria-selected={active}
              className="flex h-10 min-w-0 flex-1 items-center gap-2 px-3 text-left font-mono text-xs font-medium focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring"
              onClick={() => onActivate(tab.id)}
              onKeyDown={(event) => moveFocus(event, tab.id)}
              ref={(node) => {
                if (node) {
                  tabRefs.current?.set(tab.id, node)
                  if (active) {
                    revealTab(node)
                  }
                } else {
                  tabRefs.current?.delete(tab.id)
                }
              }}
              role="tab"
              tabIndex={tab.id === tabStopId ? 0 : -1}
              title={accessibleLabel}
              type="button"
            >
              <FileCode2 className="size-3.5 shrink-0" strokeWidth={1.7} />
              <span className="truncate">{visibleLabel}</span>
            </button>
            <button
              aria-label={`Close ${accessibleLabel}`}
              className={cn(
                'workspace-tab-close mr-1.5 flex size-6 shrink-0 items-center justify-center rounded text-muted-foreground transition-[color,background-color,opacity] hover:bg-muted hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring sm:opacity-0 sm:group-hover/tab:opacity-100 sm:focus-visible:opacity-100',
                active && 'sm:opacity-60',
              )}
              onClick={() => closeTab(tab.id)}
              type="button"
            >
              <X className="size-3.5" />
            </button>
          </div>
        )
      })}
    </div>
  )
}

function revealTab(button: HTMLButtonElement) {
  const tab = button.parentElement
  const tabList = tab?.parentElement
  if (!tab || !tabList) return

  if (tab.offsetLeft < tabList.scrollLeft) {
    tabList.scrollLeft = tab.offsetLeft
  } else if (
    tab.offsetLeft + tab.offsetWidth >
    tabList.scrollLeft + tabList.clientWidth
  ) {
    tabList.scrollLeft = tab.offsetLeft + tab.offsetWidth - tabList.clientWidth
  }
}
