import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

const RUN_WIDTHS = [82, 64, 72, 58, 70]
const PENDING_ACTIONS = (
  <div className="flex items-center gap-2">
    <Skeleton className="h-8 w-36" />
    <Skeleton className="h-8 w-28" />
  </div>
)

export function RunsPagePending() {
  return (
    <PendingSurface label="Loading runs">
      <WorkbenchPane>
        <WorkbenchBar actions={PENDING_ACTIONS} title="Runs" />
        <div className="min-w-0 border-t border-border">
          <main className="min-w-0 px-4 pb-14 sm:px-6 lg:px-8">
            <div className="divide-y divide-border pt-7">
              {RUN_WIDTHS.map((width) => (
                <div className="flex items-center gap-3 px-3 py-3" key={width}>
                  <Skeleton className="size-3.5 shrink-0 rounded-full" />
                  <Skeleton className="h-4" style={{ width: `${width}%` }} />
                </div>
              ))}
            </div>
          </main>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}
