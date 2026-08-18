import type { RepoParams, RepoRunHistoryPage } from '@/api/types'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { EmptyState } from '@/components/empty-state'
import { LoaderCircle, TerminalSquare } from 'lucide-react'
import { runDisplayState } from './run-formatting'
import { RunStatusDot } from './run-status-dot'
import { RunTimestamp } from './run-timestamp'

export function RunHistoryList({
  loadMore,
  loadingMore,
  params,
  runs,
  selectedWorkflowName,
  showLoadMore,
}: {
  loadMore: () => void
  loadingMore: boolean
  params: RepoParams
  runs: RepoRunHistoryPage['runs']
  selectedWorkflowName?: string
  showLoadMore: boolean
}) {
  return (
    <section aria-labelledby="run-history-heading" className="pt-8">
      <div className="flex items-center gap-2">
        <TerminalSquare className="size-4 text-muted-foreground" />
        <h2 className="text-sm font-semibold" id="run-history-heading">
          {selectedWorkflowName ? `${selectedWorkflowName} history` : 'All workflow runs'}
        </h2>
        <span className="text-xs tabular-nums text-muted-foreground">
          {runs.length}
        </span>
      </div>
      <div className="mt-2 divide-y divide-border">
        {runs.length === 0 ? (
          <EmptyState
            description="Push to main with a matching trigger, or run a workflow manually from the CLI."
            icon={<TerminalSquare />}
            title={selectedWorkflowName ? `No ${selectedWorkflowName} runs yet` : 'No runs yet'}
          />
        ) : runs.map((run) => {
          const state = runDisplayState(run)
          return (
            <Link
              className="group flex min-w-0 items-center gap-3 rounded-md py-3 outline-none transition-colors hover:bg-accent/50 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
              key={run.id}
              params={{ ...params, runId: run.id }}
              to="/$owner/$repo/runs/$runId"
            >
              <RunStatusDot state={state} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">
                  {run.workflow_name}
                </span>
                <span className="mt-1 block truncate font-mono text-[11px] text-muted-foreground">
                  {run.git_oid.slice(0, 12)} · Scope Cloud
                </span>
              </span>
              <span className="shrink-0 text-right text-xs text-muted-foreground">
                <span className="block capitalize text-foreground">{state}</span>
                <span className="block">
                  <RunTimestamp value={run.updated_at_unix} />
                </span>
              </span>
            </Link>
          )
        })}
      </div>
      {showLoadMore ? (
        <div className="flex justify-center pt-5">
          <Button disabled={loadingMore} onClick={loadMore} variant="secondary">
            {loadingMore ? <LoaderCircle className="animate-spin" /> : null}
            Load older runs
          </Button>
        </div>
      ) : null}
    </section>
  )
}
