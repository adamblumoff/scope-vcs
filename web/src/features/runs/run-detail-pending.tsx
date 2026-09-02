import { WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import {
  BlockSkeleton,
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import { RUN_JOB_ITEM_CLASS, RUN_JOB_STRIP_CLASS } from './run-job-layout'
import { RUN_STEP_ROW_CLASS } from './run-step-layout'

const PENDING_JOBS: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'short' },
  { id: 'second', length: 'tiny' },
  { id: 'third', length: 'medium' },
]
const PENDING_STEPS: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'medium' },
  { id: 'second', length: 'long' },
  { id: 'third', length: 'short' },
]

export function RunDetailPagePending() {
  return (
    <PendingSurface label="Loading run details">
      <WorkbenchPane>
        <header className="px-5 pb-5 pt-7 sm:px-6 lg:px-8">
          <TextSkeleton length="medium" size="meta" />
          <div className="mt-2 flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
            <div className="min-w-0">
              <TextSkeleton length="long" size="heading" />
              <TextSkeleton className="mt-3" length="medium" size="meta" />
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <BlockSkeleton className="h-9 w-24" />
              <BlockSkeleton className="h-9 w-28" />
            </div>
          </div>
        </header>
        <main className="px-4 pb-14 sm:px-6 lg:px-8">
          <section className="pt-7">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="text-sm font-semibold">Jobs</span>
              <div className="flex items-center gap-3">
                <TextSkeleton length="short" size="meta" />
                <BlockSkeleton className="h-8 w-16" />
              </div>
            </div>
            <div className={`mt-3 ${RUN_JOB_STRIP_CLASS}`}>
              {PENDING_JOBS.map((job) => (
                <div className={RUN_JOB_ITEM_CLASS} key={job.id}>
                  <BlockSkeleton className="size-3.5 rounded-full" />
                  <TextSkeleton length={job.length} />
                  <TextSkeleton length="tiny" size="meta" />
                </div>
              ))}
            </div>
            <div className="mt-6 divide-y divide-border border-t border-border">
              {PENDING_STEPS.map((step) => (
                <div
                  className={RUN_STEP_ROW_CLASS}
                  key={step.id}
                >
                  <BlockSkeleton className="size-3.5 rounded-full" />
                  <TextSkeleton length={step.length} />
                  <TextSkeleton length="tiny" size="meta" />
                </div>
              ))}
            </div>
          </section>
        </main>
      </WorkbenchPane>
    </PendingSurface>
  )
}
