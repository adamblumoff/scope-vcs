import { PageContent } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'

const REQUEST_WIDTHS = [
  { meta: 46, title: 34 },
  { meta: 38, title: 24 },
]

export function RequestsPagePending() {
  return (
    <PendingSurface label="Loading requests">
      <PageContent className="pb-16">
        <h1 className="sr-only">Requests</h1>
        <Skeleton className="h-10 w-full sm:max-w-lg" />
        <div className="mt-10 grid min-w-0 gap-12">
          {['Your work', 'Open', 'Closed'].map((section) => (
            <section className="min-w-0" key={section}>
              <div className="flex items-center gap-2">
                <Skeleton className="size-4" />
                <span className="text-sm font-semibold">{section}</span>
                <Skeleton className="h-3 w-7" />
              </div>
              <div className="mt-2 min-w-0 divide-y divide-border">
                {REQUEST_WIDTHS.map(({ meta, title }) => (
                  <div className="min-w-0 py-3" key={title}>
                    <Skeleton className="h-4" style={{ width: `${title}ch` }} />
                    <Skeleton className="mt-2 h-3" style={{ width: `${meta}ch` }} />
                  </div>
                ))}
              </div>
            </section>
          ))}
        </div>
      </PageContent>
    </PendingSurface>
  )
}
