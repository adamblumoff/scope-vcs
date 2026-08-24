import { WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

const JOB_WIDTHS = [104, 88, 120]
const STEP_WIDTHS = [42, 58, 36]

export function RunDetailPagePending() {
  return (
    <PendingSurface label="Loading run details">
      <WorkbenchPane>
        <header className="px-5 pb-5 pt-7 sm:px-6 lg:px-8">
          <Skeleton className="h-3 w-40" />
          <Skeleton className="mt-4 h-8 w-80 max-w-4/5" />
          <Skeleton className="mt-3 h-3 w-64 max-w-full" />
        </header>
        <main className="px-4 pb-14 sm:px-6 lg:px-8">
          <section className="pt-7">
            <div className="flex items-center justify-between gap-4">
              <span className="text-sm font-semibold">Jobs</span>
              <Skeleton className="h-3 w-28" />
            </div>
            <div className="mt-3 flex gap-2 border-y border-border py-3">
              {JOB_WIDTHS.map((width) => (
                <Skeleton className="h-9" key={width} style={{ width }} />
              ))}
            </div>
            <div className="mt-6 divide-y divide-border border-t border-border">
              {STEP_WIDTHS.map((width) => (
                <div className="flex items-center gap-3 py-4" key={width}>
                  <Skeleton className="size-3.5 rounded-full" />
                  <Skeleton className="h-4" style={{ width: `${width}%` }} />
                </div>
              ))}
            </div>
          </section>
        </main>
      </WorkbenchPane>
    </PendingSurface>
  )
}
