import { PageContent } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

export function RepoSettingsPending() {
  return (
    <PendingSurface label="Loading repository settings">
      <PageContent>
        <h1 className="sr-only">Settings</h1>
        <div className="divide-y divide-border">
          {['danger', 'invite', 'members'].map((row) => (
            <section
              className="grid gap-4 py-5 md:grid-cols-[240px_minmax(0,1fr)]"
              key={row}
            >
              <div>
                <Skeleton className="h-4 w-32" />
                <Skeleton className="mt-2 h-3 w-48 max-w-full" />
                <Skeleton className="mt-1.5 h-3 w-36 max-w-4/5" />
              </div>
              <div className="space-y-3 md:pt-0.5">
                {row === 'danger' ? (
                  <Skeleton className="h-8 w-40" />
                ) : row === 'invite' ? (
                  <>
                    <div className="flex gap-2">
                      <Skeleton className="h-10 min-w-0 flex-1" />
                      <Skeleton className="h-10 w-24 shrink-0" />
                    </div>
                    <Skeleton className="h-16 w-full" />
                  </>
                ) : (
                  <div className="divide-y divide-border border-y border-border">
                    {[20, 16].map((width) => (
                      <div
                        className="flex items-center justify-between gap-3 py-3"
                        key={width}
                      >
                        <div className="min-w-0 flex-1">
                          <Skeleton className="h-4" style={{ width: `${width}ch` }} />
                          <Skeleton className="mt-2 h-3 w-48" />
                        </div>
                        <Skeleton className="h-6 w-12 shrink-0" />
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </section>
          ))}
        </div>
      </PageContent>
    </PendingSurface>
  )
}
