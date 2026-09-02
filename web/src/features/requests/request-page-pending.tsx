import { WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import {
  BlockSkeleton,
  LineSkeleton,
  TextSkeleton,
  type LineSkeletonLength,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import {
  REQUEST_DESCRIPTION_CONTENT_CLASS,
  REQUEST_DISCUSSION_CONTENT_CLASS,
} from './request-content-layout'

const PENDING_THREADS: { id: string; length: LineSkeletonLength }[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'medium' },
  { id: 'third', length: 'long' },
]
const PENDING_CHANGES: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'medium' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'medium' },
]
const PENDING_DIFF_LINES: { id: string; length: LineSkeletonLength }[] = [
  { id: 'first', length: 'long' },
  { id: 'second', length: 'short' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'medium' },
  { id: 'fifth', length: 'long' },
  { id: 'sixth', length: 'short' },
]

export function RequestDetailPagePending() {
  return (
    <PendingSurface label="Loading request">
      <WorkbenchPane>
        <header className="px-5 pb-5 pt-7 sm:px-6 lg:px-8">
          <div className="flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
            <div className="min-w-0">
              <TextSkeleton length="xlong" size="heading" />
              <div className="mt-3 flex gap-2">
                <BlockSkeleton className="h-5 w-20 rounded-full" />
                <BlockSkeleton className="h-5 w-28 rounded-full" />
                <TextSkeleton length="short" />
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <BlockSkeleton className="h-8 w-24" />
              <BlockSkeleton className="h-8 w-9" />
            </div>
          </div>
        </header>
        <div className="grid min-h-0 xl:grid-cols-[minmax(0,1fr)_320px]">
          <div className="min-w-0">
            <div className="px-5 py-5 lg:px-7">
              <TextSkeleton length="short" />
              <div className={`mt-4 space-y-2 ${REQUEST_DESCRIPTION_CONTENT_CLASS}`}>
                <LineSkeleton length="full" />
                <LineSkeleton length="long" />
                <LineSkeleton length="medium" />
              </div>
            </div>
            <div className="flex h-11 gap-6 border-b border-border px-5 lg:px-7">
              <BlockSkeleton className="h-7 w-24" />
              <BlockSkeleton className="h-7 w-20" />
            </div>
            <DiscussionSkeleton />
          </div>
          <aside className="border-t border-border px-5 py-5 xl:border-l xl:border-t-0">
            <TextSkeleton length="short" />
            <BlockSkeleton className="mt-4 h-12 w-full" />
            <BlockSkeleton className="mt-4 h-12 w-full" />
            <BlockSkeleton className="mt-4 h-12 w-full" />
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
          {PENDING_CHANGES.map((change) => (
            <div className="px-5 py-4" key={change.id}>
              <TextSkeleton length={change.length} />
              <TextSkeleton className="mt-2" length="short" size="meta" />
            </div>
          ))}
        </div>
        <div className="min-h-[340px] p-5 lg:p-6">
          <TextSkeleton length="long" />
          <TextSkeleton className="mt-2" length="medium" size="meta" />
          <div className="mt-6 space-y-3">
            {PENDING_DIFF_LINES.map((line) => (
              <LineSkeleton key={line.id} length={line.length} />
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
      {PENDING_THREADS.map((thread) => (
        <article className="py-5" key={thread.id}>
          <div className="flex items-center gap-2">
            <BlockSkeleton className="size-7 rounded-full" />
            <TextSkeleton length="short" size="meta" />
          </div>
          <div className={`mt-4 space-y-2 ${REQUEST_DISCUSSION_CONTENT_CLASS}`}>
            <LineSkeleton length={thread.length} />
            <LineSkeleton length="medium" />
          </div>
        </article>
      ))}
    </section>
  )
}
