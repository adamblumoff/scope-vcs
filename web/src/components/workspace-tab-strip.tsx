import { cn } from '@/lib/utils'
import { X } from 'lucide-react'
import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react'
import {
  workspaceTabDomIds,
  workspaceTabVisibleLabels,
  type WorkspaceTabItem,
} from './workspace-tab-model'

export function WorkspaceTabStrip({
  activeId,
  ariaLabel,
  meta,
  onActivate,
  onClose,
  onEmptyFocus,
  onPin,
  previewId,
  tabSetId,
  tabs,
}: {
  activeId: string | null
  ariaLabel: string
  meta?: ReactNode
  onActivate: (id: string) => void
  onClose: (id: string) => string | null
  onEmptyFocus: () => void
  onPin: (id: string) => void
  previewId: string | null
  tabSetId: string
  tabs: WorkspaceTabItem[]
}) {
  const tabListRef = useRef<HTMLDivElement>(null)
  const tabRefs = useMemo(() => new Map<string, HTMLButtonElement>(), [])
  const refCallbacks = useMemo(
    () => new Map<string, (node: HTMLButtonElement | null) => void>(),
    [],
  )
  const tabStopId = tabs.some((tab) => tab.id === activeId)
    ? activeId
    : tabs[0]?.id
  const visibleLabels = useMemo(() => workspaceTabVisibleLabels(tabs), [tabs])
  const hiddenTabs = useStripOverflow({ activeId, tabListRef, tabRefs, tabs })

  function tabRef(id: string) {
    let callback = refCallbacks.get(id)
    if (!callback) {
      callback = (node) => {
        if (node) tabRefs.set(id, node)
        else tabRefs.delete(id)
      }
      refCallbacks.set(id, callback)
    }
    return callback
  }

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
    tabRefs?.get(tabs[nextIndex].id)?.focus()
  }

  function closeTab(id: string) {
    const focusId = onClose(id)
    requestAnimationFrame(() => {
      if (focusId) tabRefs?.get(focusId)?.focus()
      else onEmptyFocus()
    })
  }

  return (
    <div className="flex min-h-10 items-stretch gap-1 border-b border-border pr-3">
      <div
        aria-label={ariaLabel}
        className="scrollbar-none flex min-w-0 flex-1 items-stretch overflow-x-auto"
        ref={tabListRef}
        role="tablist"
      >
        {tabs.map((tab) => {
          const active = tab.id === activeId
          const accessibleLabel = tab.title ?? tab.label
          const domIds = workspaceTabDomIds(tabSetId, tab.id)
          return (
            <div
              className="group/tab relative flex shrink-0 items-center"
              key={tab.id}
              onAuxClick={(event) => {
                if (event.button !== 1) return
                event.preventDefault()
                closeTab(tab.id)
              }}
              role="presentation"
            >
              <button
                aria-controls={domIds.panelId}
                aria-label={accessibleLabel}
                aria-selected={active}
                className={cn(
                  'flex h-10 min-w-0 max-w-[200px] items-center gap-2 py-2 pl-3 text-left font-mono text-xs text-muted-foreground transition-colors hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring',
                  active && 'text-foreground',
                )}
                id={domIds.tabId}
                onClick={() => onActivate(tab.id)}
                onDoubleClick={() => onPin(tab.id)}
                onKeyDown={(event) => moveFocus(event, tab.id)}
                ref={tabRef(tab.id)}
                role="tab"
                tabIndex={tab.id === tabStopId ? 0 : -1}
                title={accessibleLabel}
                type="button"
              >
                {/* Buttons reset font-style, so the preview marker lives on the
                    label. The slanted last glyph needs room inside the clip. */}
                <span className={cn('truncate', tab.id === previewId && 'italic pr-0.5')}>
                  {visibleLabels.get(tab.id) ?? tab.label}
                </span>
              </button>
              <button
                aria-label={`Close ${accessibleLabel}`}
                className={cn(
                  'mr-1.5 flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground transition-[color,background-color,opacity] hover:bg-muted hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring sm:opacity-0 sm:group-hover/tab:opacity-100 sm:focus-visible:opacity-100',
                  active && 'sm:opacity-60',
                )}
                onClick={() => closeTab(tab.id)}
                type="button"
              >
                <X className="size-3.5" />
              </button>
              {active && (
                <span
                  aria-hidden
                  className="absolute inset-x-1.5 bottom-0 h-0.5 bg-[var(--platinum-bright)]"
                />
              )}
            </div>
          )
        })}
      </div>
      {hiddenTabs.length > 0 && (
        <OverflowMenu
          onActivate={onActivate}
          tabs={hiddenTabs}
          visibleLabels={visibleLabels}
        />
      )}
      {meta && (
        <div className="flex shrink-0 items-center gap-2 pl-2 font-mono text-[11px] text-muted-foreground">
          {meta}
        </div>
      )}
    </div>
  )
}

function OverflowMenu({
  onActivate,
  tabs,
  visibleLabels,
}: {
  onActivate: (id: string) => void
  tabs: WorkspaceTabItem[]
  visibleLabels: Map<string, string>
}) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (!open) return
    function closeOnOutsidePointer(event: MouseEvent) {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false)
      }
    }
    document.addEventListener('mousedown', closeOnOutsidePointer)
    return () =>
      document.removeEventListener('mousedown', closeOnOutsidePointer)
  }, [open])

  function closeOnEscape(event: React.KeyboardEvent) {
    if (event.key !== 'Escape') return
    setOpen(false)
    triggerRef.current?.focus()
  }

  // A disclosure rather than an ARIA menu: the buttons below sit next in tab
  // order, so they need no roving focus of their own.
  return (
    <div className="relative flex shrink-0 items-center" ref={rootRef}>
      <button
        aria-expanded={open}
        aria-label={`${tabs.length} more open ${tabs.length === 1 ? 'file' : 'files'}`}
        className="rounded border border-border px-2 py-1 font-mono text-[11px] text-muted-foreground transition-colors hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-ring"
        onClick={() => setOpen(!open)}
        onKeyDown={closeOnEscape}
        ref={triggerRef}
        type="button"
      >
        +{tabs.length}
      </button>
      {open && (
        <div className="absolute right-0 top-full z-20 mt-1 min-w-[200px] max-w-[320px] border border-border bg-popover py-1 shadow-[var(--shadow-pop)]">
          {tabs.map((tab) => (
            <button
              className="flex w-full items-center px-3 py-1.5 text-left font-mono text-xs text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring"
              key={tab.id}
              onClick={() => {
                setOpen(false)
                onActivate(tab.id)
              }}
              onKeyDown={closeOnEscape}
              title={tab.title ?? tab.label}
              type="button"
            >
              <span className="truncate">{visibleLabels.get(tab.id) ?? tab.label}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

/**
 * Keeps the active tab in view and reports the tabs scrolled out of it, so the
 * overflow menu can reach them. The strip is re-measured on resize because file
 * metadata appearing beside the tabs narrows it after the active tab changes.
 */
function useStripOverflow({
  activeId,
  tabListRef,
  tabRefs,
  tabs,
}: {
  activeId: string | null
  tabListRef: React.RefObject<HTMLDivElement | null>
  tabRefs: Map<string, HTMLButtonElement>
  tabs: WorkspaceTabItem[]
}) {
  const [hiddenIds, setHiddenIds] = useState<string[]>([])

  useLayoutEffect(() => {
    const list = tabListRef.current
    if (!list) return

    function tabRect(id: string) {
      return tabRefs.get(id)?.parentElement?.getBoundingClientRect()
    }

    function measure() {
      if (!list) return
      const bounds = list.getBoundingClientRect()
      const hidden: string[] = []
      for (const tab of tabs) {
        const rect = tabRect(tab.id)
        if (!rect) continue
        if (rect.left < bounds.left - 1 || rect.right > bounds.right + 1) {
          hidden.push(tab.id)
        }
      }
      setHiddenIds((current) =>
        current.length === hidden.length &&
        current.every((id, index) => id === hidden[index])
          ? current
          : hidden,
      )
    }

    function revealActive() {
      if (!list || !activeId) return
      const rect = tabRect(activeId)
      const bounds = list.getBoundingClientRect()
      if (!rect) return
      if (rect.left < bounds.left) list.scrollLeft -= bounds.left - rect.left
      else if (rect.right > bounds.right) list.scrollLeft += rect.right - bounds.right
    }

    revealActive()
    measure()
    const observer = new ResizeObserver(() => {
      revealActive()
      measure()
    })
    observer.observe(list)
    list.addEventListener('scroll', measure, { passive: true })
    return () => {
      observer.disconnect()
      list.removeEventListener('scroll', measure)
    }
  }, [activeId, tabListRef, tabRefs, tabs])

  return useMemo(
    () => tabs.filter((tab) => hiddenIds.includes(tab.id)),
    [hiddenIds, tabs],
  )
}
