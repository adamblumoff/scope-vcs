import type { RepoOperations, RepoParams } from '@/api/types'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RouteErrorContent } from '@/components/route-error-page'
import { Button } from '@/components/ui/button'
import { WorkbenchHeader } from '@/components/workbench-header'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import {
  ArrowRight,
  LoaderCircle,
  Server,
  TerminalSquare,
} from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { formatRunUnixTime } from './run-formatting'

const RUNS_REFRESH_INTERVAL_MS = 2_000

export function RepositoryRunsPage({
  initialOperations,
  loadOperations,
  params,
}: {
  initialOperations: RepoOperations | null
  loadOperations: (input: RepoParams) => Promise<RepoOperations | null>
  params: RepoParams
}) {
  const [operations, setOperations] = useState(initialOperations)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const mountedRef = useRef(false)
  const refreshInFlightRef = useRef<Promise<void> | null>(null)
  const { owner, repo } = params
  const canRefresh = operations !== null

  const refresh = useCallback(() => {
    if (refreshInFlightRef.current) return refreshInFlightRef.current
    const request = loadOperations({ owner, repo })
      .then((next) => {
        if (!mountedRef.current) return
        setOperations(next)
        setRefreshError(null)
      })
      .catch((error: unknown) => {
        if (mountedRef.current) setRefreshError(errorMessage(error))
      })
      .finally(() => {
        if (refreshInFlightRef.current === request) {
          refreshInFlightRef.current = null
        }
      })
    refreshInFlightRef.current = request
    return request
  }, [loadOperations, owner, repo])

  useEffect(() => {
    mountedRef.current = true
    if (!canRefresh) {
      return () => {
        mountedRef.current = false
      }
    }
    const timer = window.setInterval(
      () => void refresh(),
      RUNS_REFRESH_INTERVAL_MS,
    )
    return () => {
      mountedRef.current = false
      window.clearInterval(timer)
    }
  }, [canRefresh, refresh])

  if (!operations) {
    return (
      <>
        <RunsHeader />
        <div className="px-4 pb-12 sm:px-6 lg:px-8">
          <PageErrorAlert title="Runs unavailable">
            Sign in as the owner or a repository member to view runs and
            attached runners.
          </PageErrorAlert>
        </div>
      </>
    )
  }

  return (
    <>
      <RunsHeader runCount={operations.runs.length} />
      <div className="px-4 pb-12 sm:px-6 lg:px-8">
        {refreshError ? (
          <PageErrorAlert title="Runs could not refresh">
            <div className="flex flex-wrap items-center gap-3">
              <span>{refreshError}</span>
              <Button onClick={() => void refresh()} size="sm" variant="secondary">
                Retry now
              </Button>
            </div>
          </PageErrorAlert>
        ) : null}
        <RecentRuns params={params} runs={operations.runs} />
        <Runners owner={owner} repo={repo} runners={operations.runners} />
      </div>
    </>
  )
}

function RecentRuns({
  params,
  runs,
}: {
  params: RepoParams
  runs: RepoOperations['runs']
}) {
  return (
    <section aria-labelledby="recent-runs-heading" className="pt-7 lg:pt-10">
      <div className="flex items-center gap-2">
        <TerminalSquare className="size-4 text-muted-foreground" />
        <h2 className="text-sm font-semibold" id="recent-runs-heading">
          Recent runs
        </h2>
        <span className="text-xs tabular-nums text-muted-foreground">
          {runs.length}
        </span>
      </div>
      <div className="mt-3 divide-y divide-border border-y border-border">
        {runs.length === 0 ? (
          <EmptyRunRow />
        ) : (
          runs.map((run) => {
            const state = run.cancellation_requested ? 'canceling' : run.state
            return (
              <Link
                className="group flex min-h-16 min-w-0 items-center gap-3 px-2 py-4 outline-none hover:bg-muted/35 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                key={run.id}
                params={{ ...params, runId: run.id }}
                to="/$owner/$repo/runs/$runId"
              >
                <StatusDot state={run.state} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium">
                    {run.workflow_name}
                  </span>
                  <span className="mt-1 block truncate font-mono text-[11px] text-muted-foreground">
                    {run.git_oid.slice(0, 12)} · {run.desired_runner ?? 'any runner'}
                  </span>
                </span>
                <span className="shrink-0 text-right text-xs text-muted-foreground">
                  <span className="block capitalize text-foreground">{state}</span>
                  <span className="block">
                    {formatRunUnixTime(run.updated_at_unix)}
                  </span>
                </span>
                <ArrowRight className="size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
              </Link>
            )
          })
        )}
      </div>
    </section>
  )
}

function Runners({
  owner,
  repo,
  runners,
}: {
  owner: string
  repo: string
  runners: RepoOperations['runners']
}) {
  return (
    <section aria-labelledby="runners-heading" className="pt-9">
      <div className="flex items-center gap-2">
        <Server className="size-4 text-muted-foreground" />
        <h2 className="text-sm font-semibold" id="runners-heading">
          Runners
        </h2>
        <span className="text-xs tabular-nums text-muted-foreground">
          {runners.length}
        </span>
      </div>
      <div className="mt-3 divide-y divide-border border-y border-border">
        {runners.length === 0 ? (
          <div className="px-2 py-5 text-sm text-muted-foreground">
            <p className="font-medium text-foreground">No runners attached</p>
            <p className="mt-1">
              Attach a machine with{' '}
              <code className="break-words">
                scope runner install --name &lt;name&gt; --repo {owner}/{repo}
              </code>
              .
            </p>
          </div>
        ) : (
          runners.map((runner) => (
            <div
              className="grid gap-2 px-2 py-4 text-sm sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-center sm:gap-6"
              key={runner.id}
            >
              <div className="min-w-0">
                <div className="flex items-center gap-2 font-medium">
                  <StatusDot className="mt-0" state={runner.state} />
                  <span className="truncate">{runner.name}</span>
                </div>
                <p className="mt-1 truncate font-mono text-[11px] text-muted-foreground">
                  {runner.id}
                </p>
              </div>
              <span className="text-xs text-muted-foreground">
                v{runner.version}
              </span>
              <div className="text-xs text-muted-foreground sm:text-right">
                <div className="capitalize text-foreground">{runner.state}</div>
                <div>
                  {runner.last_seen_at_unix
                    ? `Seen ${formatRunUnixTime(runner.last_seen_at_unix)}`
                    : 'Never connected'}
                </div>
              </div>
            </div>
          ))
        )}
      </div>
    </section>
  )
}

function RunsHeader({ runCount }: { runCount?: number }) {
  return (
    <WorkbenchHeader
      actions={(
        <code className="max-w-full overflow-x-auto whitespace-nowrap text-xs text-muted-foreground">
          scope run &lt;workflow&gt; --runner &lt;name&gt;
        </code>
      )}
      count={runCount === undefined
        ? undefined
        : `${runCount} recent ${runCount === 1 ? 'run' : 'runs'}`}
      description={(
        <>
          Jobs from <code>.scope/runs</code> on your attached machines.
        </>
      )}
      eyebrow="Execute"
      title="Runs"
    />
  )
}

export function RunsPagePending() {
  return (
    <>
      <RunsHeader />
      <output
        aria-busy="true"
        className="flex items-center gap-2 px-4 py-10 text-sm text-muted-foreground sm:px-6 lg:px-8"
      >
        <LoaderCircle className="size-4 animate-spin" />
        Loading runs
      </output>
    </>
  )
}

export function RunsPageError({ error }: { error: unknown }) {
  return (
    <>
      <RunsHeader />
      <RouteErrorContent
        error={error}
        fallbackMessage="Unexpected runs error"
        title="Runs unavailable"
      />
    </>
  )
}

function EmptyRunRow() {
  return (
    <div className="px-2 py-7">
      <p className="text-sm font-medium">No runs yet</p>
      <p className="mt-1 text-sm text-muted-foreground">
        Push to main with a matching trigger, or run a workflow manually from the CLI.
      </p>
    </div>
  )
}

export function RunStatusDot({
  className,
  state,
}: {
  className?: string
  state: string
}) {
  return <StatusDot className={className} state={state} />
}

function StatusDot({
  className,
  state,
}: {
  className?: string
  state: string
}) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'size-2 shrink-0 rounded-full',
        ['online', 'running', 'succeeded'].includes(state) && 'bg-emerald-500',
        ['queued', 'leased', 'pending'].includes(state) && 'bg-amber-500',
        ['failed', 'lost', 'offline'].includes(state) && 'bg-destructive',
        ['canceled', 'disabled', 'skipped'].includes(state) && 'bg-muted-foreground',
        className,
      )}
    />
  )
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Run operation failed.'
}
