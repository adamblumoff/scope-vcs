import type {
  RepoRunAttempt,
  RepoRunDetail,
  RepoRunJobDetail,
  RepoRunStep,
  RepoRunStepLogPage,
  RunActionInput,
  RunStepLogsInput,
} from '@/api/types'
import { WorkbenchPane } from '@/components/page-header'
import { PageErrorAlert } from '@/components/page-error-alert'
import { RouteErrorContent } from '@/components/route-error-page'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import {
  Check,
  ChevronDown,
  ChevronRight,
  Circle,
  LoaderCircle,
  RotateCcw,
  Square,
  TerminalSquare,
  X,
} from 'lucide-react'
import { useMemo } from 'react'
import {
  type StepSelection,
  type StepLogState,
  useRepositoryRunDetailController,
} from './repository-run-detail-controller'
import { RunStatusDot } from './run-status-dot'
import { RunJobGraph } from './run-job-graph'
import { runJobPanelId } from './run-job-ids'
import {
  runDisplayState,
} from './run-formatting'
import { RunTimestamp } from './run-timestamp'
import { RunAttemptEnvironment } from './run-attempt-environment'

export function RepositoryRunDetailPage({
  cancelRun,
  initialDetail,
  loadDetail,
  loadLogs,
  params,
  retryRun,
}: {
  cancelRun: () => Promise<void>
  initialDetail: RepoRunDetail
  loadDetail: () => Promise<RepoRunDetail>
  loadLogs: (input: RunStepLogsInput) => Promise<RepoRunStepLogPage>
  params: RunActionInput
  retryRun: () => Promise<void>
}) {
  const {
    actionError,
    detail,
    expandedAttempts,
    expandedJobs,
    metadataError,
    pendingAction,
    performAction,
    refreshDetail,
    refreshLogs,
    selectedLogState,
    selection,
    toggleAttempt,
    toggleJob,
    toggleStep,
  } = useRepositoryRunDetailController({
    initialDetail,
    loadDetail,
    loadLogs,
    params,
  })

  return (
    <WorkbenchPane>
      <RunHeader
        actionError={actionError}
        detail={detail}
        onCancel={() => void performAction('cancel', cancelRun)}
        onRetry={() => void performAction('retry', retryRun)}
        params={params}
        pendingAction={pendingAction}
      />
      <main className="px-4 pb-14 sm:px-6 lg:px-8">
        {metadataError ? (
          <div className="pt-5">
            <PageErrorAlert title="Run details could not refresh">
              <div className="flex flex-wrap items-center gap-3">
                <span>{metadataError}</span>
                <Button
                  onClick={() => void refreshDetail()}
                  size="sm"
                  variant="secondary"
                >
                  Retry now
                </Button>
              </div>
            </PageErrorAlert>
          </div>
        ) : null}
        <section aria-labelledby="jobs-heading" className="pt-7">
          <div className="flex flex-wrap items-baseline justify-between gap-2">
            <h2 className="text-sm font-semibold" id="jobs-heading">
              Jobs
            </h2>
            <p className="text-xs text-muted-foreground">
              {jobSummary(detail.jobs)}
            </p>
          </div>
          <div className="mt-3">
            <RunJobGraph
              expandedJobs={expandedJobs}
              jobs={detail.jobs}
              onToggleJob={toggleJob}
            />
          </div>
          {expandedJobs.size > 0 ? (
            <div className="mt-6 divide-y divide-border border-y border-border">
              {detail.jobs
                .map((jobDetail) => (
                  expandedJobs.has(jobDetail.job.key) ? (
                    <JobDetailsPanel
                      expandedAttempts={expandedAttempts}
                      jobDetail={jobDetail}
                      key={jobDetail.job.key}
                      logState={selectedLogState}
                      onLogRetry={() => {
                        if (selection) void refreshLogs(selection)
                      }}
                      onSelectStep={(attemptId, stepIndex) =>
                        toggleStep(jobDetail.job.key, attemptId, stepIndex)}
                      onToggleAttempt={toggleAttempt}
                      selection={selection}
                    />
                  ) : null
                ))}
            </div>
          ) : null}
        </section>
      </main>
    </WorkbenchPane>
  )
}

function JobDetailsPanel({
  expandedAttempts,
  jobDetail,
  logState,
  onLogRetry,
  onSelectStep,
  onToggleAttempt,
  selection,
}: {
  expandedAttempts: ReadonlySet<string>
  jobDetail: RepoRunJobDetail
  logState: StepLogState
  onLogRetry: () => void
  onSelectStep: (attemptId: string, stepIndex: number) => void
  onToggleAttempt: (attempt: RepoRunAttempt) => void
  selection: StepSelection | null
}) {
  const { job, attempts } = jobDetail
  const attemptCount = `${attempts.length} ${attempts.length === 1 ? 'attempt' : 'attempts'}`
  return (
    <article id={runJobPanelId(job.key)}>
      <div className="grid min-h-16 grid-cols-[minmax(0,1fr)] items-center gap-x-3 gap-y-1 px-2 py-4 sm:grid-cols-[minmax(0,1fr)_auto]">
        <span className="flex min-w-0 items-center gap-2">
          <RunStatusDot state={job.state} />
          <span className="truncate font-medium">{job.key}</span>
          <span className="text-xs capitalize text-muted-foreground">
            {job.state}
          </span>
        </span>
        <span className="truncate text-xs text-muted-foreground sm:text-right">
          Scope Cloud · {attemptCount} · updated{' '}
          <RunTimestamp value={job.updated_at_unix} />
        </span>
      </div>
      <div className="border-t border-border/70 pl-5 sm:pl-9">
        <div className="divide-y divide-border">
          {attempts.map((attempt) => (
            <AttemptRow
              attempt={attempt}
              expanded={expandedAttempts.has(attempt.id)}
              key={attempt.id}
              logState={logState}
              onLogRetry={onLogRetry}
              onSelectStep={(stepIndex) => onSelectStep(attempt.id, stepIndex)}
              onToggle={() => onToggleAttempt(attempt)}
              pinnedContainerImage={job.pinned_container_image}
              selectedStepIndex={
                selection?.jobKey === job.key &&
                  selection.attemptId === attempt.id
                  ? selection.stepIndex
                  : null
              }
            />
          ))}
        </div>
        {attempts.length === 0 ? (
          <p className="border-t border-border px-3 py-5 text-sm text-muted-foreground">
            {job.state === 'blocked'
              ? 'Waiting for required jobs to finish.'
              : job.state === 'queued'
                ? 'Waiting for cloud capacity.'
                : 'No attempts were created for this job.'}
          </p>
        ) : null}
      </div>
    </article>
  )
}

function RunHeader({
  actionError,
  detail,
  onCancel,
  onRetry,
  params,
  pendingAction,
}: {
  actionError: string | null
  detail: RepoRunDetail
  onCancel: () => void
  onRetry: () => void
  params: RunActionInput
  pendingAction: 'cancel' | 'retry' | null
}) {
  const run = detail.run
  const state = runDisplayState(run)
  return (
    <>
      <header className="px-5 pb-5 pt-7 sm:px-6 lg:px-8">
        <Link
          className="text-xs text-muted-foreground hover:text-foreground"
          params={{ owner: params.owner, repo: params.repo }}
          to="/$owner/$repo/runs"
        >
          ← Runs
        </Link>
        <div className="mt-2 flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <h1 className="flex flex-wrap items-center gap-3 text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[30px]">
              {run.workflow_name}
              <span className="flex items-center gap-2 text-sm font-medium capitalize text-muted-foreground">
                <RunStatusDot state={state} />
                {state}
              </span>
            </h1>
            <p className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground">
              <code>{run.git_oid.slice(0, 12)}</code>
              <span aria-hidden="true">·</span>
              <span>Scope Cloud</span>
              <span aria-hidden="true">·</span>
              <span>
                Updated <RunTimestamp value={run.updated_at_unix} />
              </span>
            </p>
          </div>
          <div className="flex shrink-0 flex-wrap items-center gap-2">
            {run.can_cancel ? (
              <Button
                disabled={pendingAction !== null}
                onClick={onCancel}
                variant="secondary"
              >
                {pendingAction === 'cancel'
                  ? <LoaderCircle className="animate-spin" />
                  : <Square />}
                Cancel
              </Button>
            ) : null}
            {run.can_retry ? (
              <Button
                disabled={pendingAction !== null}
                onClick={onRetry}
                variant="secondary"
              >
                {pendingAction === 'retry'
                  ? <LoaderCircle className="animate-spin" />
                  : <RotateCcw />}
                Run again
              </Button>
            ) : null}
          </div>
        </div>
      </header>
      {actionError ? (
        <div className="px-4 pt-5 sm:px-6 lg:px-8">
          <PageErrorAlert title="Run action failed">{actionError}</PageErrorAlert>
        </div>
      ) : null}
    </>
  )
}

function AttemptRow({
  attempt,
  expanded,
  logState,
  onLogRetry,
  onSelectStep,
  onToggle,
  pinnedContainerImage,
  selectedStepIndex,
}: {
  attempt: RepoRunAttempt
  expanded: boolean
  logState: StepLogState
  onLogRetry: () => void
  onSelectStep: (stepIndex: number) => void
  onToggle: () => void
  pinnedContainerImage: string
  selectedStepIndex: number | null
}) {
  const panelId = `run-attempt-${attempt.id}`
  const stateLabel = attemptStateLabel(attempt)
  const metadata = [
    attempt.execution_provider === 'northflank' ? 'Northflank' : attempt.execution_provider,
    durationLabel('queued', attempt.created_at_unix, attempt.started_at_unix),
    durationLabel('ran', attempt.started_at_unix, attempt.completed_at_unix),
  ].filter(Boolean).join(' · ')
  return (
    <article>
      <button
        aria-controls={panelId}
        aria-expanded={expanded}
        className="grid min-h-16 w-full grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-1 px-2 py-4 text-left outline-none hover:bg-muted/35 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring sm:grid-cols-[auto_minmax(0,1fr)_auto]"
        onClick={onToggle}
        type="button"
      >
        {expanded
          ? <ChevronDown className="size-4 text-muted-foreground" />
          : <ChevronRight className="size-4 text-muted-foreground" />}
        <span className="flex min-w-0 items-center gap-2">
          <RunStatusDot state={attempt.state} />
          <span className="font-medium capitalize">{stateLabel}</span>
          <span className="truncate text-xs text-muted-foreground">
            <RunTimestamp value={attempt.created_at_unix} />
          </span>
        </span>
        <span className="col-start-2 truncate text-xs text-muted-foreground sm:col-start-3 sm:text-right">
          {metadata}
        </span>
      </button>
      {expanded ? (
        <div className="border-t border-border/70 pl-5 sm:pl-9" id={panelId}>
          {attempt.terminal_reason?.kind === 'runtime-setup-failed' ? (
            <p className="border-b border-border px-3 py-4 text-sm text-muted-foreground">
              {attempt.terminal_reason.message}
            </p>
          ) : null}
          <RunAttemptEnvironment
            caches={attempt.caches}
            pinnedContainerImage={pinnedContainerImage}
          />
          <div className="divide-y divide-border">
            {attempt.steps.map((step) => (
              <StepRow
                attemptId={attempt.id}
                key={step.index}
                logState={logState}
                onLogRetry={onLogRetry}
                onSelect={() => onSelectStep(step.index)}
                selected={selectedStepIndex === step.index}
                step={step}
              />
            ))}
          </div>
          {attempt.steps.length === 0 ? (
            <p className="border-t border-border px-3 py-5 text-sm text-muted-foreground">
              Steps are created when a runner claims this run.
            </p>
          ) : null}
        </div>
      ) : null}
    </article>
  )
}

function StepRow({
  attemptId,
  logState,
  onLogRetry,
  onSelect,
  selected,
  step,
}: {
  attemptId: string
  logState: StepLogState
  onLogRetry: () => void
  onSelect: () => void
  selected: boolean
  step: RepoRunStep
}) {
  const panelId = `run-step-${attemptId}-${step.index}`
  return (
    <div>
      <button
        aria-controls={panelId}
        aria-expanded={selected}
        className="grid min-h-14 w-full grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 px-3 py-3 text-left outline-none hover:bg-muted/30 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        onClick={onSelect}
        type="button"
      >
        <StepIcon state={step.state} />
        <span className="min-w-0">
          <span className="block truncate text-sm font-medium">{step.name}</span>
          <code className="mt-0.5 block truncate text-[11px] text-muted-foreground">
            {step.command}
          </code>
        </span>
        <span className="text-xs capitalize text-muted-foreground">
          {stepDuration(step)}
        </span>
      </button>
      {selected ? (
        <StepLogs
          id={panelId}
          logState={logState}
          onRetry={onLogRetry}
          step={step}
        />
      ) : null}
    </div>
  )
}

function StepLogs({
  id,
  logState,
  onRetry,
  step,
}: {
  id: string
  logState: StepLogState
  onRetry: () => void
  step: RepoRunStep
}) {
  return (
    <section
      aria-label={`${step.name} output`}
      className="border-t border-border bg-background text-foreground"
      id={id}
    >
      <div className="flex min-h-10 flex-wrap items-center justify-between gap-2 border-b border-border px-4 py-2 text-xs text-muted-foreground">
        <span className="flex items-center gap-2">
          <TerminalSquare className="size-3.5" />
          {step.name}
          {step.exit_code !== null ? ` · exit ${step.exit_code}` : ''}
        </span>
        <span>
          {logState.loading ? 'Updating output…' : 'Output current'}
          {logState.logsTruncated ? ' · Some output omitted' : ''}
        </span>
      </div>
      {logState.error ? (
        <div
          className="flex flex-wrap items-center gap-3 border-b border-border px-4 py-3 text-sm text-danger-strong"
          role="alert"
        >
          <span>{logState.error}</span>
          <Button onClick={onRetry} size="sm" variant="secondary">
            Retry logs
          </Button>
        </div>
      ) : null}
      <pre className="max-h-[34rem] overflow-auto whitespace-pre-wrap break-words px-4 py-4 font-mono text-xs leading-5">
        {logState.logs.length === 0
          ? <span className="text-muted-foreground">No output yet.</span>
          : logState.logs.map((log) => log.text).join('')}
      </pre>
    </section>
  )
}

function StepIcon({ state }: { state: string }) {
  const iconClass = 'size-4 shrink-0'
  if (state === 'succeeded') {
    return <Check aria-label="Succeeded" className={cn(iconClass, 'text-emerald-600')} />
  }
  if (state === 'failed') {
    return <X aria-label="Failed" className={cn(iconClass, 'text-destructive')} />
  }
  if (state === 'running') {
    return <LoaderCircle aria-label="Running" className={cn(iconClass, 'animate-spin text-warning')} />
  }
  if (state === 'canceled' || state === 'lost') {
    return <Square aria-label={capitalize(state)} className={cn(iconClass, 'text-muted-foreground')} />
  }
  return <Circle aria-label={capitalize(state)} className={cn(iconClass, 'text-muted-foreground')} />
}

function attemptStateLabel(attempt: RepoRunAttempt) {
  if (!attempt.terminal_reason) return attempt.state
  switch (attempt.terminal_reason.kind) {
    case 'timed-out':
      return 'timed out'
    case 'runtime-setup-failed':
      return 'setup failed'
    case 'execution-lost':
      return 'execution lost'
    default:
      return attempt.state
  }
}

function durationLabel(
  label: string,
  start: number | null,
  end: number | null,
) {
  if (start === null || end === null) return null
  return `${label} ${formatDuration(end - start)}`
}

function stepDuration(step: RepoRunStep) {
  if (step.state === 'skipped') return 'skipped'
  if (step.started_at_unix === null || step.completed_at_unix === null) {
    return step.state
  }
  return formatDuration(step.completed_at_unix - step.started_at_unix)
}

function formatDuration(seconds: number) {
  const safe = Math.max(0, seconds)
  if (safe < 60) return `${safe}s`
  const minutes = Math.floor(safe / 60)
  const remaining = safe % 60
  return remaining === 0 ? `${minutes}m` : `${minutes}m ${remaining}s`
}

function capitalize(value: string) {
  return value.charAt(0).toUpperCase() + value.slice(1)
}

function jobSummary(jobs: readonly RepoRunJobDetail[]) {
  const counts = new Map<string, number>()
  for (const { job } of jobs) {
    counts.set(job.state, (counts.get(job.state) ?? 0) + 1)
  }
  return [...counts.entries()]
    .map(([state, count]) => `${count} ${state}`)
    .join(' · ') || 'No jobs'
}

export function RunDetailPageError({ error }: { error: unknown }) {
  return (
    <WorkbenchPane>
      <RouteErrorContent
        error={error}
        fallbackMessage="Unexpected run detail error"
        title="Run unavailable"
      />
    </WorkbenchPane>
  )
}
