import type {
  RepoLiveState,
  RepoParams,
  RequestChanges,
  RequestSummary,
  RequestWorkflowResolutionDisposition,
  ReviewFileDiff,
} from '@/api/types'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { DestructiveActionDialog } from '@/components/destructive-action-dialog'
import { PageContent, PageHeader } from '@/components/page-header'
import { RepoShell } from '@/components/repo-shell'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import {
  CheckCircle2,
  Coins,
  GitBranch,
  GitCommitHorizontal,
  GitMerge,
  MessageSquare,
  Reply,
  Send,
  ShieldQuestion,
  Trash2,
} from 'lucide-react'
import { type FormEvent, type ReactNode } from 'react'
import { RequestChangesSection } from './request-changes-section'
import { RequestMergeDialog } from './request-merge-dialog'
import {
  RequestReviewNavigation,
  type RequestReviewView,
} from './request-review-navigation'
import { RequestTimeline } from './request-timeline'
import {
  dispositionLabel,
  dispositionTone,
  formatUnixDate,
  fullOid,
  requestAuthorRoleLabel,
  requestAudienceLabel,
  requestMergeabilityLabel,
  requestMergeabilityTone,
  requestStatusLabel,
  requestStatusTone,
  resolutionOptionsFor,
  settlementPreviewFor,
  settlementPreviewText,
  shortOid,
} from './request-labels'
import {
  type RequestActionError,
  type RequestActionKey,
  type RequestDetailControllerProps,
  useRequestDetailController,
} from './use-request-detail-controller'

export function RequestUnavailablePage({ params }: { params: RepoParams }) {
  return (
    <RepoShell params={params}>
      <PageContent>
        <PageHeader
          actions={(
            <Button asChild size="sm" variant="secondary">
              <Link params={params} to="/repos/$owner/$repo/requests">
                Requests
              </Link>
            </Button>
          )}
          badges={<Badge variant="warning">Unavailable</Badge>}
          description="This request does not exist, was deleted, or is private to repository maintainers."
          title="Request not found"
        />

        <section className="mt-8 border-t border-border py-8">
          <div className="flex max-w-2xl items-start gap-3 text-sm leading-6 text-muted-foreground">
            <ShieldQuestion className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
            <p>
              Private request branches are available only to repository
              maintainers. Sign in with an account that has access, or return
              to the request list for visible requests.
            </p>
          </div>
        </section>
      </PageContent>
    </RepoShell>
  )
}

type RequestDetailPageProps = RequestDetailControllerProps & {
  changes: RequestChanges | null
  changesError: string | null
  live: RepoLiveState
  onSelectFile: (path: string) => void
  onViewChange: (view: RequestReviewView) => void
  selectedDiff: ReviewFileDiff | null
  selectedDiffError: string | null
  selectedPath: string | null
  view: RequestReviewView
}

export function RequestDetailPage(props: RequestDetailPageProps) {
  const {
    changes,
    changesError,
    detail,
    live,
    onSelectFile,
    onViewChange,
    params,
    selectedDiff,
    selectedDiffError,
    selectedPath,
    view,
  } = props
  const {
    activeResolveDisposition,
    deleteOpen,
    request,
    resolutionOptions,
    setCommentBody,
    setDeleteOpen,
    setMergeOpen,
    setNeedsResponseBody,
    setResolveBody,
    setResolveDisposition,
    setResponseBody,
    submitComment,
    submitDelete,
    submitMerge,
    submitNeedsResponse,
    submitResolution,
    submitResponse,
    uiState,
  } = useRequestDetailController(props)

  return (
    <RepoShell params={params}>
      <PageContent>
        <RequestDetailHeader
          live={live}
          onDelete={() => setDeleteOpen(true)}
          params={params}
          pendingAction={uiState.pendingAction}
          request={request}
        />
        <RequestReviewNavigation onChange={onViewChange} view={view} />

        {view === 'overview' && (
          <>
            <RequestFacts request={request} />

            <RequestGitInstructions request={request} />

            <RequestActions
              activeResolveDisposition={activeResolveDisposition}
              actionError={uiState.actionError}
              commentBody={uiState.commentBody}
              needsResponseBody={uiState.needsResponseBody}
              onCommentBodyChange={setCommentBody}
              onMergeOpen={() => setMergeOpen(true)}
              onNeedsResponseBodyChange={setNeedsResponseBody}
              onResolveBodyChange={setResolveBody}
              onResolveDispositionChange={setResolveDisposition}
              onResponseBodyChange={setResponseBody}
              onSubmitComment={submitComment}
              onSubmitNeedsResponse={submitNeedsResponse}
              onSubmitResolution={submitResolution}
              onSubmitResponse={submitResponse}
              pendingAction={uiState.pendingAction}
              request={request}
              resolutionOptions={resolutionOptions}
              resolveBody={uiState.resolveBody}
              responseBody={uiState.responseBody}
            />
          </>
        )}

        {view === 'changes' && (
          <RequestChangesSection
            changes={changes}
            error={changesError}
            onSelectFile={onSelectFile}
            selectedDiff={selectedDiff}
            selectedDiffError={selectedDiffError}
            selectedPath={selectedPath}
          />
        )}

        {view === 'activity' && <RequestTimeline detail={detail} />}
      </PageContent>

      <RequestMergeDialog
        error={errorFor(uiState.actionError, 'merge')}
        onConfirm={submitMerge}
        onOpenChange={setMergeOpen}
        open={uiState.mergeOpen}
        pending={uiState.pendingAction === 'merge'}
        request={request}
      />
      <DestructiveActionDialog
        confirmLabel={request.state === 'Working' ? 'Delete request' : 'Withdraw request'}
        description={
          request.state === 'Working'
            ? 'This removes the request branch from Scope. Local Git branches are not deleted.'
            : 'This closes the request and removes it from maintainer review. Its activity remains visible.'
        }
        onConfirm={() => void submitDelete()}
        onOpenChange={(open) => {
          if (uiState.pendingAction !== 'delete') setDeleteOpen(open)
        }}
        open={deleteOpen}
        pending={uiState.pendingAction === 'delete'}
        subject={request.title}
        title={request.state === 'Working' ? 'Delete working request?' : 'Withdraw request?'}
      />
    </RepoShell>
  )
}

function RequestDetailHeader({
  live,
  onDelete,
  params,
  pendingAction,
  request,
}: {
  live: RepoLiveState
  onDelete: () => void
  params: RepoParams
  pendingAction: RequestActionKey | null
  request: RequestSummary
}) {
  return (
    <PageHeader
      actions={(
        <div className="flex flex-wrap items-center gap-2">
          {request.permissions.can_delete ? (
            <Button
              disabled={pendingAction === 'delete'}
              onClick={onDelete}
              size="sm"
              variant="destructive"
            >
              <Trash2 className="size-4" />
              Delete
            </Button>
          ) : null}
          <Button asChild size="sm" variant="secondary">
            <Link params={params} to="/repos/$owner/$repo/requests">
              Requests
            </Link>
          </Button>
        </div>
      )}
      badges={(
        <>
          <LifecycleBadge state={live.repo.lifecycle_state} />
          <Badge variant={requestStatusTone(request)}>
            {requestStatusLabel(request)}
          </Badge>
          <Badge variant={requestMergeabilityTone(request)}>
            {requestMergeabilityLabel(request)}
          </Badge>
          <Badge variant="neutral">
            <Coins className="size-3" />
            <span>{request.stake_credits}</span>
          </Badge>
        </>
      )}
      description={(
        <span className="font-mono text-sm">
          {request.name} / {request.id}
        </span>
      )}
      title={request.title}
    />
  )
}

function RequestFacts({ request }: { request: RequestSummary }) {
  return (
    <SectionRows>
      <SectionRow
        columns="compact"
        description="The stable named branch carried by every authorized clone and pull."
        icon={<GitBranch className="size-4 text-muted-foreground" />}
        title="Branch"
      >
        <div className="grid gap-2 text-sm leading-5">
          <KeyValue label="Remote branch" value={`origin/${request.name}`} />
          <div className="flex flex-wrap gap-1.5">
            <Badge variant="outline">{requestAudienceLabel(request)}</Badge>
            <Badge variant="outline">{requestAuthorRoleLabel(request)}</Badge>
          </div>
        </div>
      </SectionRow>

      <SectionRow
        columns="compact"
        description="Exact Git positions used for stale confirmation checks."
        icon={<GitCommitHorizontal className="size-4 text-muted-foreground" />}
        title="Git state"
      >
        <div className="grid gap-2 text-sm leading-5">
          <KeyValue label="Base main" value={fullOid(request.base_main_oid)} />
          <KeyValue label="Request head" value={fullOid(request.head_oid)} />
          <KeyValue
            label="Current main"
            value={fullOid(request.mergeability.current_main_oid)}
          />
        </div>
      </SectionRow>

      <SectionRow
        columns="compact"
        description="Stake settlement is calculated from the maintainer disposition."
        icon={<Coins className="size-4 text-muted-foreground" />}
        title="Credits"
      >
        {request.settlement ? (
          <div className="grid gap-2 text-sm leading-5">
            <div className="flex flex-wrap gap-1.5">
              <Badge variant={dispositionTone(request.settlement.disposition)}>
                {dispositionLabel(request.settlement.disposition)}
              </Badge>
              <Badge variant="neutral">
                settled {formatUnixDate(request.settlement.settled_at_unix)}
              </Badge>
            </div>
            <div className="font-mono text-xs tabular-nums text-muted-foreground">
              {settlementPreviewText({
                burnedCredits: request.settlement.burned_credits,
                refundedCredits: request.settlement.refunded_credits,
                rewardCredits: request.settlement.reward_credits,
                stakeCredits: request.settlement.stake_credits,
              })}
            </div>
          </div>
        ) : (
          <div className="font-mono text-xs tabular-nums text-muted-foreground">
            {request.stake_credits} staked / pending settlement
          </div>
        )}
      </SectionRow>
    </SectionRows>
  )
}

function RequestGitInstructions({ request }: { request: RequestSummary }) {
  return (
    <section className="mt-8 border-t border-border pt-6">
      <div className="flex items-start gap-3">
        <GitBranch className="mt-1 size-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0">
          <h2 className="text-balance text-lg font-semibold leading-7">
            Work on this request
          </h2>
          <p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">
            Scope clone and pull include every request available to you. Fetch,
            then check out the named remote branch with ordinary Git.
          </p>
          <pre className="mt-3 overflow-x-auto rounded-md bg-muted px-3 py-2 text-xs leading-5 text-foreground"><code>{`git fetch origin\ngit switch --track origin/${request.name}`}</code></pre>
        </div>
      </div>
    </section>
  )
}

function RequestActions({
  activeResolveDisposition,
  actionError,
  commentBody,
  needsResponseBody,
  onCommentBodyChange,
  onMergeOpen,
  onNeedsResponseBodyChange,
  onResolveBodyChange,
  onResolveDispositionChange,
  onResponseBodyChange,
  onSubmitComment,
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
  commentBody: string
  needsResponseBody: string
  onCommentBodyChange: (value: string) => void
  onMergeOpen: () => void
  onNeedsResponseBodyChange: (value: string) => void
  onResolveBodyChange: (value: string) => void
  onResolveDispositionChange: (value: RequestWorkflowResolutionDisposition) => void
  onResponseBodyChange: (value: string) => void
  onSubmitComment: (event: FormEvent<HTMLFormElement>) => void
  onSubmitNeedsResponse: (event: FormEvent<HTMLFormElement>) => void
  onSubmitResolution: (event: FormEvent<HTMLFormElement>) => void
  onSubmitResponse: (event: FormEvent<HTMLFormElement>) => void
  pendingAction: RequestActionKey | null
  request: RequestSummary
  resolutionOptions: ReturnType<typeof resolutionOptionsFor>
  resolveBody: string
  responseBody: string
}) {
  const hasActions =
    request.permissions.can_comment ||
    request.permissions.can_mark_needs_response ||
    request.permissions.can_respond ||
    request.permissions.can_merge ||
    request.permissions.can_resolve

  if (!hasActions) {
    return null
  }

  return (
    <section className="mt-8">
      <h2 className="text-balance text-lg font-semibold leading-7">Actions</h2>
      <SectionRows className="mt-2">
        {request.permissions.can_comment && (
          <SectionRow
            columns="compact"
            description="Add context without changing request state."
            icon={<MessageSquare className="size-4 text-muted-foreground" />}
            title="Comment"
          >
            <ActionForm onSubmit={onSubmitComment}>
              <ActionTextarea
                label="Comment body"
                onChange={onCommentBodyChange}
                placeholder="Add a note to the request timeline"
                value={commentBody}
              />
              <ActionFooter
                disabled={!commentBody.trim()}
                error={errorFor(actionError, 'comment')}
                icon={<Send className="size-3.5" />}
                label="Comment"
                pending={pendingAction === 'comment'}
                pendingLabel="Commenting"
              />
            </ActionForm>
          </SectionRow>
        )}

        {request.permissions.can_mark_needs_response && (
          <SectionRow
            columns="compact"
            description="Ask the contributor for clarification or a revision."
            icon={<ShieldQuestion className="size-4 text-muted-foreground" />}
            title="Needs response"
          >
            <ActionForm onSubmit={onSubmitNeedsResponse}>
              <ActionTextarea
                label="Needs-response body"
                onChange={onNeedsResponseBodyChange}
                placeholder="What does the contributor need to answer?"
                value={needsResponseBody}
              />
              <ActionFooter
                disabled={!needsResponseBody.trim()}
                error={errorFor(actionError, 'needs-response')}
                icon={<ShieldQuestion className="size-3.5" />}
                label="Request response"
                pending={pendingAction === 'needs-response'}
                pendingLabel="Requesting"
              />
            </ActionForm>
          </SectionRow>
        )}

        {request.permissions.can_respond && (
          <SectionRow
            columns="compact"
            description="Return the request to submitted after maintainer follow-up."
            icon={<Reply className="size-4 text-muted-foreground" />}
            title="Respond"
          >
            <ActionForm onSubmit={onSubmitResponse}>
              <ActionTextarea
                label="Response body"
                onChange={onResponseBodyChange}
                placeholder="Optional response note"
                value={responseBody}
              />
              <ActionFooter
                error={errorFor(actionError, 'respond')}
                icon={<Reply className="size-3.5" />}
                label="Respond"
                pending={pendingAction === 'respond'}
                pendingLabel="Responding"
              />
            </ActionForm>
          </SectionRow>
        )}

        {request.permissions.can_merge && (
          <MergeActionRow
            error={errorFor(actionError, 'merge')}
            onMergeOpen={onMergeOpen}
            pending={pendingAction === 'merge'}
            request={request}
          />
        )}

        {request.permissions.can_resolve && (
          <SectionRow
            columns="compact"
            description="Close without merge. Accepted requests must use merge."
            icon={<CheckCircle2 className="size-4 text-muted-foreground" />}
            title="Resolve"
          >
            <ActionForm onSubmit={onSubmitResolution}>
              <div className="grid gap-2">
                <select
                  aria-label="Resolution disposition"
                  className={cn(
                    'h-9 w-full rounded-lg border border-input bg-background',
                    'px-3 text-sm outline-none focus-visible:border-ring',
                    'focus-visible:ring-3 focus-visible:ring-ring/50',
                  )}
                  onChange={(event) =>
                    onResolveDispositionChange(
                      event.target.value as RequestWorkflowResolutionDisposition,
                    )
                  }
                  value={activeResolveDisposition}
                >
                  {resolutionOptions.map((option) => (
                    <option
                      key={option.disposition}
                      value={option.disposition}
                    >
                      {option.label}
                    </option>
                  ))}
                </select>
                <p className="text-pretty text-xs leading-5 text-muted-foreground">
                  {
                    resolutionOptions.find(
                      (option) =>
                        option.disposition === activeResolveDisposition,
                    )?.description
                  }
                </p>
                <div className="font-mono text-xs tabular-nums text-muted-foreground">
                  {settlementPreviewText(
                    settlementPreviewFor(request, activeResolveDisposition),
                  )}
                </div>
                <ActionTextarea
                  label="Resolution body"
                  onChange={onResolveBodyChange}
                  placeholder="Optional resolution note"
                  value={resolveBody}
                />
              </div>
              <ActionFooter
                error={errorFor(actionError, 'resolve')}
                icon={<CheckCircle2 className="size-3.5" />}
                label="Resolve"
                pending={pendingAction === 'resolve'}
                pendingLabel="Resolving"
                variant="secondary"
              />
            </ActionForm>
          </SectionRow>
        )}
      </SectionRows>
    </section>
  )
}

function MergeActionRow({
  error,
  onMergeOpen,
  pending,
  request,
}: {
  error: string | null
  onMergeOpen: () => void
  pending: boolean
  request: RequestSummary
}) {
  const mergeReady =
    request.mergeability.status === 'Ready' &&
    Boolean(request.mergeability.current_main_oid) &&
    Boolean(request.mergeability.request_head_oid)
  const reason =
    request.mergeability.reason ??
    (!request.mergeability.current_main_oid
      ? 'Request has no current main OID to merge into.'
      : !request.mergeability.request_head_oid
        ? 'Request has no current branch head OID to merge.'
        : 'Only clean request branches can be merged.')

  return (
    <SectionRow
      columns="compact"
      description="Merge cleanly into the current main branch and settle as accepted."
      icon={<GitMerge className="size-4 text-muted-foreground" />}
      title="Merge"
    >
      <div className="grid gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <Button
            disabled={!mergeReady || pending}
            onClick={onMergeOpen}
            size="sm"
            type="button"
            variant="success"
          >
            <GitMerge className="size-3.5" />
            <span>{pending ? 'Merging…' : 'Merge request'}</span>
          </Button>
          <Badge variant={requestMergeabilityTone(request)}>
            {requestMergeabilityLabel(request)}
          </Badge>
        </div>
        {!mergeReady && (
          <p className="text-pretty text-sm leading-5 text-muted-foreground">
            {reason}
          </p>
        )}
        {error && <ActionErrorMessage>{error}</ActionErrorMessage>}
      </div>
    </SectionRow>
  )
}

function ActionForm({
  children,
  onSubmit,
}: {
  children: ReactNode
  onSubmit: (event: FormEvent<HTMLFormElement>) => void
}) {
  return (
    <form className="grid gap-3" onSubmit={onSubmit}>
      {children}
    </form>
  )
}

function ActionTextarea({
  label,
  onChange,
  placeholder,
  value,
}: {
  label: string
  onChange: (value: string) => void
  placeholder: string
  value: string
}) {
  return (
    <textarea
      aria-label={label}
      className={cn(
        'min-h-24 w-full resize-y rounded-lg border border-input',
        'bg-background px-3 py-2 text-sm leading-5 outline-none',
        'placeholder:text-muted-foreground focus-visible:border-ring',
        'focus-visible:ring-3 focus-visible:ring-ring/50',
      )}
      onChange={(event) => onChange(event.target.value)}
      placeholder={placeholder}
      value={value}
    />
  )
}

function ActionFooter({
  disabled = false,
  error,
  icon,
  label,
  pending,
  pendingLabel,
  variant = 'default',
}: {
  disabled?: boolean
  error: string | null
  icon: ReactNode
  label: string
  pending: boolean
  pendingLabel: string
  variant?: React.ComponentProps<typeof Button>['variant']
}) {
  return (
    <div className="grid gap-2">
      <div>
        <Button disabled={disabled || pending} size="sm" type="submit" variant={variant}>
          {icon}
          <span>{pending ? `${pendingLabel}…` : label}</span>
        </Button>
      </div>
      {error && <ActionErrorMessage>{error}</ActionErrorMessage>}
    </div>
  )
}

function ActionErrorMessage({ children }: { children: ReactNode }) {
  return <p className="text-sm leading-5 text-destructive" role="alert">{children}</p>
}

function KeyValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid gap-1">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <span className="break-all font-mono text-xs leading-5">{value}</span>
    </div>
  )
}

function errorFor(
  actionError: RequestActionError | null,
  key: RequestActionKey,
) {
  return actionError?.key === key ? actionError.message : null
}
