import type { RepoRunAttempt, RepoRunJobDetail, RepoRunStep } from '@/api/types'
import { cn } from '@/lib/utils'
import type { StepLogState, StepSelection } from './repository-run-detail-controller'
import { RunAttemptEnvironment } from './run-attempt-environment'
import { RunDuration } from './run-duration'
import { RunLogView } from './run-log-view'
import { RunStatusIcon } from './run-status-icon'
import { RUN_STEP_ROW_CLASS } from './run-step-layout'
import { runStatus } from './run-status'

/** The steps of a single job attempt: an attempt switcher only when more than
 * one attempt exists, then the attempt's environment facts and step list. */
export function RunDetailSteps({
  attempt,
  jobDetail,
  onLogRetry,
  onSelectAttempt,
  onSelectStep,
  selectedLogState,
  selection,
}: {
  attempt: RepoRunAttempt | null
  jobDetail: RepoRunJobDetail
  onLogRetry: () => void
  onSelectAttempt: (attemptId: string) => void
  onSelectStep: (attemptId: string, stepIndex: number) => void
  selectedLogState: StepLogState
  selection: StepSelection | null
}) {
  const { attempts, job } = jobDetail
  const terminalNotice = attempt ? attemptTerminalNotice(attempt) : null
  return (
    <div>
      {attempts.length > 1 ? (
        <AttemptSwitcher
          attempts={attempts}
          onSelect={onSelectAttempt}
          selectedAttemptId={attempt?.id ?? null}
        />
      ) : null}
      {attempt ? (
        <>
          {terminalNotice ? (
            <p className="flex items-center gap-2 border-b border-border px-1 py-4 text-sm">
              <RunStatusIcon
                state={attempt.state}
                terminalReason={attempt.terminal_reason}
              />
              <span className="font-medium">{terminalNotice.label}</span>
              {terminalNotice.message ? (
                <span className="text-muted-foreground">{terminalNotice.message}</span>
              ) : null}
            </p>
          ) : null}
          <RunAttemptEnvironment
            caches={attempt.caches}
            cacheSetup={attempt.cache_setup}
            pinnedContainerImage={job.pinned_container_image}
          />
          <div className="divide-y divide-border">
            {attempt.steps.map((step) => (
              <StepRow
                attemptId={attempt.id}
                key={step.index}
                onLogRetry={onLogRetry}
                onSelect={() => onSelectStep(attempt.id, step.index)}
                selected={selection?.jobKey === job.key &&
                  selection.attemptId === attempt.id &&
                  selection.stepIndex === step.index}
                selectedLogState={selectedLogState}
                step={step}
              />
            ))}
          </div>
          {attempt.steps.length === 0 ? (
            <p className="border-t border-border px-1 py-5 text-sm text-muted-foreground">
              Steps are created when a runner claims this run.
            </p>
          ) : null}
        </>
      ) : (
        <p className="border-t border-border px-1 py-5 text-sm text-muted-foreground">
          {job.state === 'blocked'
            ? 'Waiting for required jobs to finish.'
            : job.state === 'queued'
              ? 'Waiting for cloud capacity.'
              : 'No attempts were created for this job.'}
        </p>
      )}
    </div>
  )
}

function attemptTerminalNotice(attempt: RepoRunAttempt) {
  const reason = attempt.terminal_reason
  if (!reason) return null
  const status = runStatus(attempt.state, reason)
  if (status.label === attempt.state) return null
  return {
    label: status.label,
    message: reason.kind === 'runtime-setup-failed' ? reason.message : null,
  }
}

function AttemptSwitcher({
  attempts,
  onSelect,
  selectedAttemptId,
}: {
  attempts: readonly RepoRunAttempt[]
  onSelect: (attemptId: string) => void
  selectedAttemptId: string | null
}) {
  // The API returns attempts newest first; a switcher reads better in run order.
  const orderedAttempts = [...attempts].sort((left, right) =>
    left.number - right.number)
  return (
    <div
      aria-label="Attempts"
      className="flex flex-wrap items-center gap-1.5 border-b border-border py-3"
    >
      {orderedAttempts.map((attempt) => (
        <button
          aria-pressed={attempt.id === selectedAttemptId}
          className={cn(
            'flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring',
            attempt.id === selectedAttemptId
              ? 'border-foreground/40 bg-muted'
              : 'border-border text-muted-foreground hover:bg-muted/50',
          )}
          key={attempt.id}
          onClick={() => onSelect(attempt.id)}
          type="button"
        >
          <RunStatusIcon state={attempt.state} terminalReason={attempt.terminal_reason} />
          Attempt {attempt.number}
        </button>
      ))}
    </div>
  )
}

function StepRow({
  attemptId,
  onLogRetry,
  onSelect,
  selected,
  selectedLogState,
  step,
}: {
  attemptId: string
  onLogRetry: () => void
  onSelect: () => void
  selected: boolean
  selectedLogState: StepLogState
  step: RepoRunStep
}) {
  const panelId = `run-step-${attemptId}-${step.index}`
  return (
    <div>
      <button
        aria-controls={panelId}
        aria-expanded={selected}
        className={`${RUN_STEP_ROW_CLASS} text-left outline-none hover:bg-muted/30 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring`}
        onClick={onSelect}
        type="button"
      >
        <RunStatusIcon state={step.state} />
        <span className="min-w-0">
          <span className="block truncate text-sm font-medium">{step.name}</span>
          <code className="mt-0.5 block truncate text-[11px] text-muted-foreground">
            {step.command}
          </code>
        </span>
        <span className="text-xs text-muted-foreground">
          {step.state === 'skipped'
            ? 'Skipped'
            : <RunDuration end={step.completed_at_unix} start={step.started_at_unix} />}
        </span>
      </button>
      {selected ? (
        <RunLogView
          id={panelId}
          key={panelId}
          logState={selectedLogState}
          onRetry={onLogRetry}
          step={step}
        />
      ) : null}
    </div>
  )
}
