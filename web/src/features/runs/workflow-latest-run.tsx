import type {
  RepoRunDetail,
  RepoRunHistoryPage,
  RepoRunJobDetail,
  RunActionInput,
} from '@/api/types'
import { PageErrorAlert } from '@/components/page-error-alert'
import { PendingSurface } from '@/components/pending-surface'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { reconcileExpandedJobs, runNeedsPolling } from './repository-run-detail-model'
import { RunStatusDot } from './run-status-dot'
import { RunJobGraph } from './run-job-graph'
import { runJobPanelId } from './run-job-ids'
import { runDisplayState } from './run-formatting'
import { RunTimestamp } from './run-timestamp'

const LATEST_RUN_REFRESH_INTERVAL_MS = 2_000

export function WorkflowLatestRun({
  loadDetail,
  params,
  run,
}: {
  loadDetail: (input: RunActionInput) => Promise<RepoRunDetail>
  params: { owner: string; repo: string }
  run: RepoRunHistoryPage['runs'][number] | undefined
}) {
  const [detail, setDetail] = useState<RepoRunDetail | null>(null)
  const [expandedJobs, setExpandedJobs] = useState<Set<string>>(new Set())
  const [error, setError] = useState<string | null>(null)
  const mountedRef = useRef(false)
  const inFlightRef = useRef<Promise<void> | null>(null)
  const { owner, repo } = params
  const runId = run?.id
  const detailState = detail?.run.state
  const input = useMemo(
    () => runId ? { owner, repo, run_id: runId } : null,
    [owner, repo, runId],
  )

  const refresh = useCallback(() => {
    if (!input) return
    if (inFlightRef.current) return inFlightRef.current
    const request = loadDetail(input)
      .then((next) => {
        if (!mountedRef.current) return
        setDetail(next)
        setExpandedJobs((current) => reconcileExpandedJobs(current, next.jobs))
        setError(null)
      })
      .catch((cause: unknown) => {
        if (mountedRef.current) setError(errorMessage(cause))
      })
      .finally(() => {
        if (inFlightRef.current === request) inFlightRef.current = null
      })
    inFlightRef.current = request
    return request
  }, [input, loadDetail])

  useEffect(() => {
    mountedRef.current = true
    if (!input) {
      return () => {
        mountedRef.current = false
      }
    }
    if (!detailState) void refresh()
    if (detailState && !runNeedsPolling(detailState)) {
      return () => {
        mountedRef.current = false
      }
    }
    const timer = window.setInterval(
      () => void refresh(),
      LATEST_RUN_REFRESH_INTERVAL_MS,
    )
    return () => {
      mountedRef.current = false
      window.clearInterval(timer)
    }
  }, [detailState, input, refresh])

  if (!run) return null
  const displayedRun = detail?.run ?? run
  const displayedState = runDisplayState(displayedRun)

  return (
    <section aria-labelledby="latest-run-heading" className="pt-7 lg:pt-9">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <RunStatusDot state={displayedState} />
            <h2 className="text-sm font-semibold" id="latest-run-heading">
              Latest run
            </h2>
            <span className="text-xs capitalize text-muted-foreground">
              {displayedState}
            </span>
          </div>
          <p className="mt-1 truncate text-xs text-muted-foreground">
            {run.workflow_name} · updated{' '}
            <RunTimestamp value={displayedRun.updated_at_unix} />
          </p>
        </div>
        <Link
          className="inline-flex items-center gap-1.5 text-sm font-medium text-muted-foreground hover:text-foreground"
          params={{ ...params, runId: run.id }}
          to="/$owner/$repo/runs/$runId"
        >
          View run
          <ArrowRight className="size-3.5" />
        </Link>
      </div>
      {error ? (
        <div className="pt-4">
          <PageErrorAlert title="Latest run could not refresh">
            <div className="flex flex-wrap items-center gap-3">
              <span>{error}</span>
              <Button onClick={() => void refresh()} size="sm" variant="secondary">
                Retry now
              </Button>
            </div>
          </PageErrorAlert>
        </div>
      ) : null}
      {detail ? (
        <>
          <div className="mt-3">
            <RunJobGraph
              compact
              expandedJobs={expandedJobs}
              jobs={detail.jobs}
              onToggleJob={(job) => {
                setExpandedJobs((current) => toggleSet(current, job.job.key))
              }}
            />
          </div>
          {expandedJobs.size > 0 ? (
            <div className="divide-y divide-border border-b border-border">
              {detail.jobs.map((job) => expandedJobs.has(job.job.key) ? (
                <LatestJobSummary jobDetail={job} key={job.job.key} />
              ) : null)}
            </div>
          ) : null}
        </>
      ) : (
        <PendingSurface
          className="mt-3 min-h-[140px] border-y border-border"
          label="Loading latest job graph"
        />
      )}
    </section>
  )
}

function LatestJobSummary({ jobDetail }: { jobDetail: RepoRunJobDetail }) {
  const { job, attempts } = jobDetail
  return (
    <div
      className="flex flex-wrap items-center justify-between gap-2 px-2 py-3 text-sm"
      id={runJobPanelId(job.key)}
    >
      <span className="flex items-center gap-2 font-medium">
        <RunStatusDot state={job.state} />
        {job.key}
      </span>
      <span className="text-xs text-muted-foreground">
        {jobStatusLabel(jobDetail)}
      </span>
      <span className="basis-full text-xs text-muted-foreground sm:basis-auto">
        {attempts.length} {attempts.length === 1 ? 'attempt' : 'attempts'}
      </span>
    </div>
  )
}

function jobStatusLabel({ job, attempts }: RepoRunJobDetail) {
  if (job.state === 'blocked') return `Waiting for ${job.needs.join(', ')}`
  if (job.state === 'queued') {
    return 'Queued for Scope Cloud'
  }
  if (job.state === 'dispatching') return 'Starting cloud job'
  if (job.state === 'running') {
    const activeStep = attempts
      .flatMap((attempt) => attempt.steps)
      .find((step) => step.state === 'running')
    return activeStep ? `Running ${activeStep.name}` : 'Running'
  }
  return (
    <>
      {capitalize(job.state)} <RunTimestamp value={job.updated_at_unix} />
    </>
  )
}

function toggleSet(current: ReadonlySet<string>, key: string) {
  const next = new Set(current)
  if (next.has(key)) next.delete(key)
  else next.add(key)
  return next
}

function capitalize(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1)
}

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Latest run refresh failed.'
}
