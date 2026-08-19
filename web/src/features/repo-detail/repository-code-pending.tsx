import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

const TREE_WIDTHS = [58, 72, 46, 68, 52, 76, 42]
const CODE_WIDTHS = [76, 52, 84, 64, 38, 72, 56, 80, 44, 68]
const PENDING_ACTIONS = <Skeleton className="h-8 w-24" />
const PENDING_SUMMARY = <Skeleton className="h-4 w-20" />

export function RepositoryCodePending() {
  return (
    <PendingSurface label="Loading repository files">
      <WorkbenchPane>
        <WorkbenchBar
          actions={PENDING_ACTIONS}
          summary={PENDING_SUMMARY}
          title="Code"
        />
        <div className="grid min-w-0 lg:min-h-[calc(100dvh-var(--app-chrome))] lg:grid-cols-[minmax(300px,0.36fr)_minmax(0,0.64fr)]">
          <div className="border-b border-border px-3 py-3 lg:border-b-0 lg:border-r lg:px-5">
            <Skeleton className="mb-3 hidden h-3 w-24 sm:block" />
            <div className="divide-y divide-border">
              {TREE_WIDTHS.map((width, index) => (
                <div
                  className="grid min-h-9 grid-cols-[18px_minmax(0,1fr)_64px] items-center gap-2"
                  key={`${width}-${index}`}
                >
                  <Skeleton className="size-3.5" />
                  <Skeleton className="h-3" style={{ width: `${width}%` }} />
                  <Skeleton className="h-3 w-full" />
                </div>
              ))}
            </div>
          </div>
          <div className="min-w-0">
            <div className="flex h-11 items-center border-b border-border px-3">
              <Skeleton className="h-6 w-32" />
            </div>
            <div className="space-y-3 p-5 sm:p-7">
              {CODE_WIDTHS.map((width, index) => (
                <div className="flex items-center gap-4" key={`${width}-${index}`}>
                  <Skeleton className="h-3 w-5 shrink-0" />
                  <Skeleton className="h-3" style={{ width: `${width}%` }} />
                </div>
              ))}
            </div>
          </div>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}
