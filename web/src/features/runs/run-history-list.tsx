import type { RepoParams, RepoRunHistoryPage } from '@/api/types'
import { Button } from '@/components/ui/button'
import { EmptyState } from '@/components/empty-state'
import { LoaderCircle, TerminalSquare } from 'lucide-react'
import { RunRow } from './run-row'

export function RunHistoryList({
  loadMore,
  loadingMore,
  params,
  runs,
  selectedWorkflowName,
  showLoadMore,
  totalRunCount,
}: {
  loadMore: () => void
  loadingMore: boolean
  params: RepoParams
  runs: RepoRunHistoryPage['runs']
  selectedWorkflowName?: string
  showLoadMore: boolean
  totalRunCount: number
}) {
  if (totalRunCount === 0) {
    return (
      <EmptyState
        description="Push to main with a matching trigger, or run a workflow manually from the CLI."
        icon={<TerminalSquare />}
        title={selectedWorkflowName ? `No ${selectedWorkflowName} runs yet` : 'No runs yet'}
      />
    )
  }

  if (runs.length === 0) {
    return (
      <EmptyState
        description="Try a different workflow or status filter."
        icon={<TerminalSquare />}
        title="No runs match this filter"
      />
    )
  }

  return (
    <div>
      <div className="divide-y divide-border">
        {runs.map((run) => (
          <RunRow key={run.id} params={params} run={run} />
        ))}
      </div>
      <div className="flex items-center justify-center gap-3 pt-5">
        {showLoadMore ? (
          <Button disabled={loadingMore} onClick={loadMore} variant="secondary">
            {loadingMore ? <LoaderCircle className="animate-spin" /> : null}
            Load older runs
          </Button>
        ) : null}
        <span className="text-xs text-muted-foreground">
          Showing {runs.length}
        </span>
      </div>
    </div>
  )
}
