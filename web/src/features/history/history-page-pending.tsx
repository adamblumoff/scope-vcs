import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import {
  BlockSkeleton,
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'
import { CommitDetailSkeleton } from './history-commit-detail-skeleton'
import {
  HISTORY_ENTRY_PRIMARY_CLASS,
  HISTORY_ENTRY_ROW_CLASS,
  HISTORY_ENTRY_TITLE_CLASS,
} from './history-entry-layout'

const PENDING_COMMITS: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'medium' },
  { id: 'second', length: 'short' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'medium' },
  { id: 'fifth', length: 'short' },
]
const PENDING_ACTIONS = <BlockSkeleton className="h-8 w-28" />
const PENDING_SUMMARY = <TextSkeleton length="short" />

export function HistoryPagePending() {
  return (
    <PendingSurface label="Loading repository history">
      <WorkbenchPane>
        <WorkbenchBar
          actions={PENDING_ACTIONS}
          summary={PENDING_SUMMARY}
          title="History"
        />
        <div className="grid min-w-0 grid-cols-1 border-t border-border lg:grid-cols-[minmax(260px,0.4fr)_minmax(0,1.6fr)]">
          <div className="divide-y divide-border border-b border-border lg:border-b-0 lg:border-r">
            {PENDING_COMMITS.map((commit) => (
              <div
                className={HISTORY_ENTRY_ROW_CLASS}
                key={commit.id}
              >
                <div className={HISTORY_ENTRY_PRIMARY_CLASS}>
                  <BlockSkeleton className="size-3.5 shrink-0" />
                  <div className="min-w-0">
                    <div className={HISTORY_ENTRY_TITLE_CLASS}>
                      <BlockSkeleton className="h-5 w-12 shrink-0 rounded-full" />
                      <TextSkeleton length={commit.length} size="meta" />
                    </div>
                    <TextSkeleton className="mt-1" length="short" size="meta" />
                  </div>
                </div>
                <TextSkeleton length="tiny" size="meta" />
              </div>
            ))}
          </div>
          <div className="min-w-0">
            <CommitDetailSkeleton showDiff />
          </div>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}
