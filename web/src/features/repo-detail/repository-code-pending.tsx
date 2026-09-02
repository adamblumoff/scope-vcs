import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import {
  FileNavigatorSkeleton,
  SourceCodeSkeleton,
} from './repository-code-skeletons'

const PENDING_ACTIONS = <BlockSkeleton className="h-8 w-24" />
const PENDING_SUMMARY = <TextSkeleton length="short" />

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
            <TextSkeleton
              className="mb-3 hidden sm:block"
              length="short"
              size="meta"
            />
            <FileNavigatorSkeleton />
          </div>
          <div className="min-w-0">
            <div className="flex h-11 items-center border-b border-border px-3">
              <BlockSkeleton className="h-6 w-32" />
            </div>
            <SourceCodeSkeleton />
          </div>
        </div>
      </WorkbenchPane>
    </PendingSurface>
  )
}
