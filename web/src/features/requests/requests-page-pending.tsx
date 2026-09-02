import { PageContent } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import {
  BlockSkeleton,
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'

const REQUEST_WIDTHS = [
  { meta: 'xlong', title: 'long' },
  { meta: 'long', title: 'medium' },
] satisfies Array<{ meta: TextSkeletonLength; title: TextSkeletonLength }>

export function RequestsPagePending() {
  return (
    <PendingSurface label="Loading requests">
      <PageContent className="pb-16">
        <h1 className="sr-only">Requests</h1>
        <BlockSkeleton className="h-10 w-full sm:max-w-lg" />
        <div className="mt-10 grid min-w-0 gap-12">
          {['Your work', 'Open', 'Closed'].map((section) => (
            <section className="min-w-0" key={section}>
              <div className="flex items-center gap-2">
                <BlockSkeleton className="size-4" />
                <span className="text-sm font-semibold">{section}</span>
                <TextSkeleton length="tiny" size="meta" />
              </div>
              <div className="mt-2 min-w-0 divide-y divide-border">
                {REQUEST_WIDTHS.map(({ meta, title }) => (
                  <div className="min-w-0 py-3" key={title}>
                    <TextSkeleton length={title} />
                    <TextSkeleton className="mt-2" length={meta} size="meta" />
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
