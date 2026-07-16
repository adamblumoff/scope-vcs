import type {
  RequestSummary,
  RequestWorkflowResolutionDisposition,
} from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  CheckCircle2,
  Coins,
  GitBranch,
  GitCommitHorizontal,
  GitMerge,
  Reply,
  ShieldQuestion,
} from 'lucide-react'
import type { ComponentProps, FormEvent, ReactNode } from 'react'
import {
  requestAudienceLabel,
  requestAuthorRoleLabel,
  requestMergeabilityLabel,
  requestMergeabilityTone,
  settlementPreviewFor,
  settlementPreviewText,
  shortOid,
} from './request-labels'
import type { RequestActionError, RequestActionKey } from './use-request-actions'

export function RequestContextRail({
  activeResolveDisposition,
  actionError,
  needsResponseBody,
  onMergeOpen,
  onNeedsResponseBodyChange,
  onResolveBodyChange,
  onResolveDispositionChange,
  onResponseBodyChange,
  onSubmitNeedsResponse,
  onSubmitResolution,
  onSubmitResponse,
  pendingAction,
  request,
  resolutionOptions,
  resolveBody,
  responseBody,
}: {
  activeResolveDisposition: RequestWorkflowResolutionDisposition
  actionError: RequestActionError | null
  needsResponseBody: string
  onMergeOpen: () => void
  onNeedsResponseBodyChange: (value: string) => void
  onResolveBodyChange: (value: string) => void
  onResolveDispositionChange: (
    value: RequestWorkflowResolutionDisposition,
  ) => void
  onResponseBodyChange: (value: string) => void
  onSubmitNeedsResponse: (event: FormEvent<HTMLFormElement>) => void
  onSubmitResolution: (event: FormEvent<HTMLFormElement>) => void
  onSubmitResponse: (event: FormEvent<HTMLFormElement>) => void
  pendingAction: RequestActionKey | null
  request: RequestSummary
  resolutionOptions: Array<{
    description: string
    disposition: RequestWorkflowResolutionDisposition
    label: string
  }>
  resolveBody: string
  responseBody: string
}) {
  return (
    <aside className="min-w-0 border-l border-border bg-muted/15">
      <RailSection icon={<GitBranch />} title="Request">
        <RailValue label="Branch" value={`origin/${request.name}`} />
        <div className="flex flex-wrap gap-1.5">
          <Badge variant="outline">{requestAudienceLabel(request)}</Badge>
          <Badge variant="outline">{requestAuthorRoleLabel(request)}</Badge>
        </div>
      </RailSection>

      <RailSection icon={<GitCommitHorizontal />} title="Git state">
        <RailValue label="Base" value={shortOid(request.base_main_oid)} />
        <RailValue label="Head" value={shortOid(request.head_oid)} />
        <pre className="overflow-x-auto rounded-md bg-muted px-3 py-2 text-[11px] leading-5"><code>{`git fetch origin\ngit switch --track origin/${request.name}`}</code></pre>
      </RailSection>

      <RailSection icon={<Coins />} title="Credits">
        <p className="font-mono text-xs tabular-nums text-muted-foreground">
          {request.stake_credits} staked
        </p>
        {request.settlement ? (
          <p className="font-mono text-xs leading-5 text-muted-foreground">
            {settlementPreviewText({
              burnedCredits: request.settlement.burned_credits,
              refundedCredits: request.settlement.refunded_credits,
              rewardCredits: request.settlement.reward_credits,
              stakeCredits: request.settlement.stake_credits,
            })}
          </p>
        ) : null}
      </RailSection>

      {request.permissions.can_mark_needs_response ? (
        <RailAction
          error={errorFor(actionError, 'needs-response')}
          icon={<ShieldQuestion />}
          onSubmit={onSubmitNeedsResponse}
          pending={pendingAction === 'needs-response'}
          submitLabel="Request response"
          title="Needs response"
        >
          <RailTextarea
            label="Needs-response body"
            onChange={onNeedsResponseBodyChange}
            placeholder="What needs clarification or revision?"
            required
            value={needsResponseBody}
          />
        </RailAction>
      ) : null}

      {request.permissions.can_respond ? (
        <RailAction
          error={errorFor(actionError, 'respond')}
          icon={<Reply />}
          onSubmit={onSubmitResponse}
          pending={pendingAction === 'respond'}
          submitLabel="Respond"
          title="Respond"
        >
          <RailTextarea
            label="Response body"
            onChange={onResponseBodyChange}
            placeholder="Optional response note"
            value={responseBody}
          />
        </RailAction>
      ) : null}

      {request.permissions.can_merge ? (
        <RailSection icon={<GitMerge />} title="Merge">
          <Badge variant={requestMergeabilityTone(request)}>
            {requestMergeabilityLabel(request)}
          </Badge>
          <Button
            disabled={
              request.mergeability.status !== 'Ready' ||
              pendingAction === 'merge'
            }
            onClick={onMergeOpen}
            size="sm"
            type="button"
            variant="success"
          >
            <GitMerge className="size-3.5" />
            Merge request
          </Button>
          {errorFor(actionError, 'merge') ? (
            <RailError>{errorFor(actionError, 'merge')}</RailError>
          ) : null}
        </RailSection>
      ) : null}

      {request.permissions.can_resolve ? (
        <RailAction
          error={errorFor(actionError, 'resolve')}
          icon={<CheckCircle2 />}
          onSubmit={onSubmitResolution}
          pending={pendingAction === 'resolve'}
          submitLabel="Resolve request"
          title="Resolve without merge"
          variant="secondary"
        >
          <select
            aria-label="Resolution disposition"
            className={cn(
              'h-9 w-full rounded-md border border-input bg-background px-3 text-sm',
              'outline-none focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50',
            )}
            onChange={(event) =>
              onResolveDispositionChange(
                event.target.value as RequestWorkflowResolutionDisposition,
              )
            }
            value={activeResolveDisposition}
          >
            {resolutionOptions.map((option) => (
              <option key={option.disposition} value={option.disposition}>
                {option.label}
              </option>
            ))}
          </select>
          <p className="text-xs leading-5 text-muted-foreground">
            {
              resolutionOptions.find(
                ({ disposition }) =>
                  disposition === activeResolveDisposition,
              )?.description
            }
          </p>
          <p className="font-mono text-[11px] leading-5 text-muted-foreground">
            {settlementPreviewText(
              settlementPreviewFor(request, activeResolveDisposition),
            )}
          </p>
          <RailTextarea
            label="Resolution body"
            onChange={onResolveBodyChange}
            placeholder="Optional resolution note"
            value={resolveBody}
          />
        </RailAction>
      ) : null}
    </aside>
  )
}

function RailSection({
  children,
  icon,
  title,
}: {
  children: ReactNode
  icon: ReactNode
  title: string
}) {
  return (
    <section className="border-b border-border px-5 py-5">
      <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.08em] text-muted-foreground [&_svg]:size-3.5">
        {icon}
        <h2>{title}</h2>
      </div>
      <div className="mt-3 grid gap-3">{children}</div>
    </section>
  )
}

function RailAction({
  children,
  error,
  icon,
  onSubmit,
  pending,
  submitLabel,
  title,
  variant = 'default',
}: {
  children: ReactNode
  error: string | null
  icon: ReactNode
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
  pending: boolean
  submitLabel: string
  title: string
  variant?: ComponentProps<typeof Button>['variant']
}) {
  return (
    <RailSection icon={icon} title={title}>
      <form className="grid gap-2" onSubmit={onSubmit}>
        {children}
        <div>
          <Button disabled={pending} size="sm" type="submit" variant={variant}>
            {pending ? 'Working…' : submitLabel}
          </Button>
        </div>
        {error ? <RailError>{error}</RailError> : null}
      </form>
    </RailSection>
  )
}

function RailTextarea({
  label,
  onChange,
  placeholder,
  required = false,
  value,
}: {
  label: string
  onChange: (value: string) => void
  placeholder: string
  required?: boolean
  value: string
}) {
  return (
    <textarea
      aria-label={label}
      className={cn(
        'min-h-20 w-full resize-y rounded-md border border-input bg-background',
        'px-3 py-2 text-sm leading-5 outline-none placeholder:text-muted-foreground',
        'focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50',
      )}
      onChange={(event) => onChange(event.target.value)}
      placeholder={placeholder}
      required={required}
      value={value}
    />
  )
}

function RailValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid gap-1">
      <span className="text-[11px] font-medium text-muted-foreground">
        {label}
      </span>
      <span className="break-all font-mono text-xs">{value}</span>
    </div>
  )
}

function RailError({ children }: { children: ReactNode }) {
  return (
    <p className="text-xs leading-5 text-destructive" role="alert">
      {children}
    </p>
  )
}

function errorFor(
  actionError: RequestActionError | null,
  key: RequestActionKey,
) {
  return actionError?.key === key ? actionError.message : null
}
