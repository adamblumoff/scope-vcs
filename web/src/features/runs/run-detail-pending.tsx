import { WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

const JOB_WIDTHS = [104, 88, 120]
const STEP_TITLE_WIDTHS = [18, 24, 14]

export function RunDetailPagePending() {
  return (
    <PendingSurface label="Loading run details">
      <WorkbenchPane>
        <header className="px-5 pb-5 pt-7 sm:px-6 lg:px-8">
          <Skeleton className="h-3 w-40" />
          <div className="mt-2 flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0">
              <Skeleton className="h-8 w-80" />
              <Skeleton className="mt-3 h-3 w-64" />
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <Skeleton className="h-9 w-24" />
              <Skeleton className="h-9 w-28" />
            </div>
          </div>
        </header>
        <main className="px-4 pb-14 sm:px-6 lg:px-8">
          <section className="pt-7">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="text-sm font-semibold">Jobs</span>
              <div className="flex items-center gap-3">
                <Skeleton className="h-3 w-20" />
                <Skeleton className="h-8 w-16" />
              </div>
            </div>
            <div className="mt-3 flex gap-2 border-y border-border py-3">
              {JOB_WIDTHS.map((width) => (
                <Skeleton className="h-9" key={width} style={{ width }} />
              ))}
            </div>
            <div className="mt-6 divide-y divide-border border-t border-border">
              {STEP_TITLE_WIDTHS.map((width) => (
                <div
                  className="grid min-h-14 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 px-1 py-3"
                  key={width}
                >
                  <Skeleton className="size-3.5 rounded-full" />
                  <Skeleton className="h-4" style={{ width: `${width}ch` }} />
                  <Skeleton className="h-3 w-14" />
                </div>
              ))}
            </div>
          </section>
        </main>
      </WorkbenchPane>
    </PendingSurface>
  )
}
