import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'
import { RunGraphSkeleton } from './run-graph-skeleton'

const WORKFLOW_WIDTHS = [68, 52, 76, 60]
const RUN_WIDTHS = [82, 64, 72]
const PENDING_ACTIONS = <Skeleton className="h-4 w-56 max-w-[45vw]" />
const PENDING_SUMMARY = <Skeleton className="h-4 w-24" />

export function RunsPagePending() {
  return (
    <PendingSurface label="Loading workflows and runs">
      <WorkbenchPane>
        <WorkbenchBar
          actions={PENDING_ACTIONS}
          summary={PENDING_SUMMARY}
          title="Runs"
        />
        <div className="grid min-w-0 border-t border-border lg:grid-cols-[14rem_minmax(0,1fr)]">
          <nav className="border-b border-border px-4 py-5 lg:border-b-0 lg:border-r">
            <Skeleton className="mb-4 h-3 w-20" />
            <div className="space-y-4">
              {WORKFLOW_WIDTHS.map((width) => (
                <Skeleton className="h-4" key={width} style={{ width: `${width}%` }} />
              ))}
            </div>
          </nav>
          <main className="min-w-0 px-4 pb-14 sm:px-6 lg:px-8">
            <section className="pt-7">
              <div className="flex items-center justify-between gap-4">
                <Skeleton className="h-4 w-32" />
                <Skeleton className="h-3 w-24" />
              </div>
              <RunGraphSkeleton />
            </section>
            <section className="mt-10">
              <Skeleton className="h-4 w-28" />
              <div className="mt-3 divide-y divide-border border-y border-border">
                {RUN_WIDTHS.map((width) => (
                  <div className="py-4" key={width}>
                    <Skeleton className="h-4" style={{ width: `${width}%` }} />
                    <Skeleton className="mt-2 h-3 w-40" />
                  </div>
                ))}
              </div>
            </section>
          </main>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}

export function RunDetailPagePending() {
  return (
    <PendingSurface label="Loading run details">
      <WorkbenchPane>
        <header className="px-5 pb-5 pt-7 sm:px-6 lg:px-8">
          <Skeleton className="h-3 w-24" />
          <Skeleton className="mt-4 h-8 w-80 max-w-4/5" />
          <div className="mt-3 flex gap-2">
            <Skeleton className="h-5 w-20 rounded-full" />
            <Skeleton className="h-5 w-28 rounded-full" />
          </div>
          <Skeleton className="mt-3 h-3 w-64 max-w-full" />
        </header>
        <main className="px-4 pb-14 sm:px-6 lg:px-8">
          <section className="pt-7">
            <div className="flex items-center justify-between gap-4">
              <span className="text-sm font-semibold">Jobs</span>
              <Skeleton className="h-3 w-28" />
            </div>
            <RunGraphSkeleton />
          </section>
        </main>
      </WorkbenchPane>
    </PendingSurface>
  )
}
