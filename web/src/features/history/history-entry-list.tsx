import type { HistoryEntrySummary } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { historyEntryLabels } from '@/features/history/history-row-labels'
import { cn } from '@/lib/utils'
import { History, LoaderCircle } from 'lucide-react'

export function HistoryEntryList({
  entries,
  loadOlderError,
  loadingOlder,
  onLoadOlder,
  onSelectEntry,
  selectedEntryId,
  showLoadOlder,
}: {
  entries: HistoryEntrySummary[]
  loadOlderError: string | null
  loadingOlder: boolean
  onLoadOlder: () => void
  onSelectEntry: (entry: HistoryEntrySummary) => void
  selectedEntryId: string | null
  showLoadOlder: boolean
}) {
  return (
    <div className="border-b border-border py-2 lg:border-b-0 lg:border-r">
      {entries.map((entry) => {
        const labels = historyEntryLabels(entry)
        const selected = selectedEntryId === entry.source_id
        return (
          <button
            aria-label={labels.ariaLabel}
            className={cn(
              'grid w-full grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 px-5 py-2.5 text-left text-sm transition-colors sm:px-6 lg:px-8',
              selected
                ? 'bg-accent shadow-[inset_2px_0_0_0_var(--brand)]'
                : 'hover:bg-accent/50',
            )}
            key={entry.source_id}
            onClick={() => onSelectEntry(entry)}
            title={entry.source_id}
            type="button"
          >
            <span className="flex min-w-0 items-center gap-2">
              <History className="size-3.5 shrink-0 text-muted-foreground" />
              <span className="min-w-0">
                <span className="flex min-w-0 items-center gap-2">
                  <Badge className="shrink-0" variant="neutral">{labels.kind}</Badge>
                  <span className="truncate text-[13px] font-medium">{labels.title}</span>
                </span>
                <span className="mt-0.5 block truncate font-mono text-[11px] leading-4 text-muted-foreground">
                  {labels.compactId}
                  {labels.visibilityBreakdown ? ` · ${labels.visibilityBreakdown}` : null}
                </span>
              </span>
            </span>
            <span className="font-mono text-xs tabular-nums text-muted-foreground">
              {labels.count}
            </span>
          </button>
        )
      })}
      {showLoadOlder ? (
        <div className="flex flex-col items-center gap-2 px-5 py-4">
          <Button
            disabled={loadingOlder}
            onClick={onLoadOlder}
            size="sm"
            variant="secondary"
          >
            {loadingOlder ? <LoaderCircle className="animate-spin" /> : null}
            Load older history
          </Button>
          {loadOlderError ? (
            <span className="text-center text-xs text-destructive" role="alert">
              {loadOlderError}
            </span>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
