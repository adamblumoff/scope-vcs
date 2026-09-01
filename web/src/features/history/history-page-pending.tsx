import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'
import { CommitDetailSkeleton } from './history-commit-detail-skeleton'

const COMMIT_TITLE_WIDTHS = [18, 12, 22, 16, 14]
const PENDING_ACTIONS = <Skeleton className="h-8 w-28" />
const PENDING_SUMMARY = <Skeleton className="h-4 w-24" />

export function HistoryPagePending() {
  return (
    <PendingSurface label="Loading repository history">
      <WorkbenchPane>
        <WorkbenchBar
          actions={PENDING_ACTIONS}
          summary={PENDING_SUMMARY}
          title="History"
        />
        <div className="grid min-w-0 grid-cols-1 border-t border-border lg:grid-cols-[minmax(260px,0.4fr)_minmax(0,1.6fr)]">
          <div className="divide-y divide-border border-b border-border lg:border-b-0 lg:border-r">
            {COMMIT_TITLE_WIDTHS.map((width) => (
              <div
                className="grid min-h-[60px] grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 px-5 py-2.5 sm:px-6 lg:px-8"
                key={width}
              >
                <div className="flex min-w-0 items-center gap-2">
                  <Skeleton className="size-3.5 shrink-0" />
                  <div className="min-w-0">
                    <div className="flex min-w-0 items-center gap-2">
                      <Skeleton className="h-5 w-12 shrink-0 rounded-full" />
                      <Skeleton className="h-3" style={{ width: `${width}ch` }} />
                    </div>
                    <Skeleton className="mt-1 h-3 w-32" />
                  </div>
                </div>
                <Skeleton className="h-3 w-5" />
              </div>
            ))}
          </div>
          <div className="min-w-0">
            <CommitDetailSkeleton showDiff />
          </div>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}
