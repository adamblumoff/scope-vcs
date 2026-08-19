import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

const COMMIT_WIDTHS = [72, 54, 82, 66, 60]
const FILE_WIDTHS = [58, 74, 48, 68]
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
        <div className="grid border-t border-border lg:grid-cols-[minmax(260px,0.4fr)_minmax(0,1.6fr)]">
          <div className="divide-y divide-border border-b border-border lg:border-b-0 lg:border-r">
            {COMMIT_WIDTHS.map((width) => (
              <div className="px-5 py-4" key={width}>
                <Skeleton className="h-4" style={{ width: `${width}%` }} />
                <Skeleton className="mt-2 h-3 w-32" />
              </div>
            ))}
          </div>
          <div>
            <div className="border-b border-border px-5 py-4 sm:px-6">
              <Skeleton className="h-4 w-2/3" />
              <Skeleton className="mt-2 h-3 w-48" />
            </div>
            <div className="grid xl:grid-cols-[minmax(0,0.9fr)_minmax(360px,1.1fr)]">
              <div className="divide-y divide-border">
                {FILE_WIDTHS.map((width) => (
                  <div className="flex min-h-9 items-center gap-3 px-5" key={width}>
                    <Skeleton className="size-3.5" />
                    <Skeleton className="h-3" style={{ width: `${width}%` }} />
                  </div>
                ))}
              </div>
              <div className="min-h-[340px] border-border p-5 xl:border-l">
                <Skeleton className="h-3 w-4/5" />
                <Skeleton className="mt-3 h-3 w-3/5" />
                <Skeleton className="mt-3 h-3 w-11/12" />
              </div>
            </div>
          </div>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}
