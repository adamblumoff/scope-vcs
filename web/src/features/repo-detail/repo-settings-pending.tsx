import { PageContent } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

export function RepoSettingsPending() {
  return (
    <PendingSurface label="Loading repository settings">
      <PageContent>
        <h1 className="sr-only">Settings</h1>
        <div className="divide-y divide-border">
          {[0, 1, 2].map((row) => (
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
                <Skeleton className="h-9 w-40" />
                {row > 0 ? <Skeleton className="h-12 w-full" /> : null}
              </div>
            </section>
          ))}
        </div>
      </PageContent>
    </PendingSurface>
  )
}
