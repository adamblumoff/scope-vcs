import { WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

const THREAD_WIDTHS = [82, 64, 74]
const CHANGE_WIDTHS = [72, 54, 78, 62]

export function RequestDetailPagePending() {
  return (
    <PendingSurface label="Loading request">
      <WorkbenchPane>
        <header className="px-5 pb-5 pt-7 sm:px-6 lg:px-8">
          <Skeleton className="h-8 w-[34rem] max-w-4/5" />
          <div className="mt-3 flex gap-2">
            <Skeleton className="h-5 w-20 rounded-full" />
            <Skeleton className="h-5 w-28 rounded-full" />
            <Skeleton className="h-4 w-32" />
          </div>
        </header>
        <div className="grid min-h-0 xl:grid-cols-[minmax(0,1fr)_320px]">
          <div className="min-w-0">
            <div className="px-5 py-5 lg:px-7">
              <Skeleton className="h-4 w-24" />
              <Skeleton className="mt-4 h-3 w-11/12" />
              <Skeleton className="mt-2 h-3 w-4/5" />
              <Skeleton className="mt-2 h-3 w-2/3" />
            </div>
            <div className="flex h-11 gap-6 border-b border-border px-5 lg:px-7">
              <Skeleton className="h-7 w-24" />
              <Skeleton className="h-7 w-20" />
            </div>
            <DiscussionSkeleton />
          </div>
          <aside className="border-t border-border px-5 py-5 xl:border-l xl:border-t-0">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="mt-4 h-12 w-full" />
            <Skeleton className="mt-4 h-12 w-full" />
            <Skeleton className="mt-4 h-12 w-full" />
          </aside>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}

export function RequestDiscussionPending() {
  return (
    <PendingSurface label="Loading request discussion">
      <DiscussionSkeleton />
    </PendingSurface>
  )
}

export function RequestChangesPending() {
  return (
    <PendingSurface label="Loading request changes">
      <section className="grid border-t border-border lg:grid-cols-[minmax(220px,0.42fr)_minmax(0,1.58fr)]">
        <div className="divide-y divide-border border-b border-border lg:border-b-0 lg:border-r">
          {CHANGE_WIDTHS.map((width) => (
            <div className="px-5 py-4" key={width}>
              <Skeleton className="h-4" style={{ width: `${width}%` }} />
              <Skeleton className="mt-2 h-3 w-28" />
            </div>
          ))}
        </div>
        <div className="min-h-[340px] p-5 lg:p-6">
          <Skeleton className="h-4 w-2/3" />
          <Skeleton className="mt-2 h-3 w-40" />
          <div className="mt-6 space-y-3">
            {[74, 48, 86, 62, 78, 42].map((width) => (
              <Skeleton className="h-3" key={width} style={{ width: `${width}%` }} />
            ))}
          </div>
        </div>
      </section>
    </PendingSurface>
  )
}

function DiscussionSkeleton() {
  return (
    <section className="divide-y divide-border px-5 lg:px-7">
      {THREAD_WIDTHS.map((width) => (
        <article className="py-5" key={width}>
          <div className="flex items-center gap-2">
            <Skeleton className="size-7 rounded-full" />
            <Skeleton className="h-3 w-28" />
          </div>
          <Skeleton className="mt-4 h-3" style={{ width: `${width}%` }} />
          <Skeleton className="mt-2 h-3 w-3/5" />
        </article>
      ))}
    </section>
  )
}
