import type {
  CommentRequestInput,
  MergeRequestInput,
  NeedsResponseInput,
  RepoLiveState,
  RepoParams,
  RequestDetail,
  RequestMutation,
  RequestSummary,
  RequestWorkflowDisposition,
  ResolveRequestInput,
  RespondRequestInput,
} from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { RepoBreadcrumb } from '@/components/repo-breadcrumb'
import { SectionRow, SectionRows } from '@/components/section-rows'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Link, useRouter } from '@tanstack/react-router'
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
} from 'lucide-react'
import { type FormEvent, type ReactNode, useReducer } from 'react'
import { RequestMergeDialog } from './request-merge-dialog'
import {
  dispositionLabel,
  dispositionTone,
  eventKindLabel,
  formatUnixDate,
  fullOid,
  normalizedBody,
  requestAuthorRoleLabel,
  requestBaseAudienceLabel,
  requestEventBody,
  requestMergeabilityLabel,
  requestMergeabilityTone,
  requestStateLabel,
  requestStateTone,
  resolutionOptionsFor,
  settlementPreviewFor,
  settlementPreviewText,
  shortOid,
} from './request-labels'

type RequestMutationAction<TInput> = (input: TInput) => Promise<RequestMutation>

type ActionKey = 'comment' | 'merge' | 'needs-response' | 'resolve' | 'respond'

type ActionError = {
  key: ActionKey
  message: string
}

type RequestBodyField =
  | 'commentBody'
  | 'needsResponseBody'
  | 'resolveBody'
  | 'responseBody'

type RequestDetailUiState = {
  actionError: ActionError | null
  commentBody: string
  mergeOpen: boolean
  needsResponseBody: string
  pendingAction: ActionKey | null
  resolveBody: string
  resolveDisposition: RequestWorkflowDisposition
  responseBody: string
}

type RequestDetailUiAction =
  | { field: RequestBodyField; type: 'bodyChanged'; value: string }
  | { disposition: RequestWorkflowDisposition; type: 'resolveDispositionChanged' }
  | { open: boolean; type: 'mergeOpenChanged' }
  | { key: ActionKey; type: 'actionStarted' }
  | {
      closeMerge?: boolean
      resetField?: RequestBodyField
      type: 'actionSucceeded'
    }
  | { key: ActionKey; message: string; type: 'actionFailed' }

const initialRequestDetailUiState: RequestDetailUiState = {
  actionError: null,
  commentBody: '',
  mergeOpen: false,
  needsResponseBody: '',
  pendingAction: null,
  resolveBody: '',
  resolveDisposition: 'UsefulNotMerged',
  responseBody: '',
}

export function RequestDetailPage({
  commentRequest,
  detail,
  live,
  markNeedsResponse,
  mergeRequest,
  params,
  resolveRequest,
  respondToRequest,
}: {
  commentRequest: RequestMutationAction<CommentRequestInput>
  detail: RequestDetail
  live: RepoLiveState
  markNeedsResponse: RequestMutationAction<NeedsResponseInput>
  mergeRequest: RequestMutationAction<MergeRequestInput>
  params: RepoParams
  resolveRequest: RequestMutationAction<ResolveRequestInput>
  respondToRequest: RequestMutationAction<RespondRequestInput>
}) {
  const router = useRouter()
  const { request } = detail
  const [uiState, dispatch] = useReducer(
    requestDetailUiReducer,
    initialRequestDetailUiState,
  )
  const resolutionOptions = resolutionOptionsFor(request)
  const activeResolveDisposition = resolutionOptions.some(
    (option) => option.disposition === uiState.resolveDisposition,
  )
    ? uiState.resolveDisposition
    : resolutionOptions[0]?.disposition ?? 'UsefulNotMerged'
  const requestParams = { ...params, request_id: request.id }

  async function runAction(
    key: ActionKey,
    action: () => Promise<unknown>,
    success?: { closeMerge?: boolean; resetField?: RequestBodyField },
  ) {
    dispatch({ key, type: 'actionStarted' })
    try {
      await action()
      await router.invalidate()
      dispatch({ type: 'actionSucceeded', ...success })
    } catch (error) {
      dispatch({
        key,
        message: error instanceof Error ? error.message : 'request action failed',
        type: 'actionFailed',
      })
    }
  }

  async function submitComment(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'comment',
      () => commentRequest({ ...requestParams, body: uiState.commentBody }),
      { resetField: 'commentBody' },
    )
  }

  async function submitNeedsResponse(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'needs-response',
      () =>
        markNeedsResponse({
          ...requestParams,
          body: uiState.needsResponseBody,
        }),
      { resetField: 'needsResponseBody' },
    )
  }

  async function submitResponse(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'respond',
      () =>
        respondToRequest({
          ...requestParams,
          body: normalizedBody(uiState.responseBody),
        }),
      { resetField: 'responseBody' },
    )
  }

  async function submitResolution(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'resolve',
      () =>
        resolveRequest({
          ...requestParams,
          body: normalizedBody(uiState.resolveBody),
          disposition: activeResolveDisposition,
        }),
      { resetField: 'resolveBody' },
    )
  }

  async function submitMerge(body: string | null) {
    const currentMainOid = request.mergeability.current_main_oid
    if (!currentMainOid) {
      dispatch({
        key: 'merge',
        message: 'Request has no current main OID to merge into.',
        type: 'actionFailed',
      })
      return
    }

    await runAction(
      'merge',
      () =>
        mergeRequest({
          ...requestParams,
          body,
          expected_head_oid: request.mergeability.request_head_oid,
          expected_main_oid: currentMainOid,
        }),
      { closeMerge: true },
    )
  }

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader
        breadcrumb={() => <RepoBreadcrumb params={params} section="requests" />}
      />

      <PageContent>
        <PageHeader
          actions={() => (
            <Button asChild size="sm" variant="secondary">
              <Link params={params} to="/repos/$owner/$repo/requests">
                Requests
              </Link>
            </Button>
          )}
          badges={() => (
            <>
              <LifecycleBadge state={live.repo.lifecycle_state} />
              <Badge variant={requestStateTone(request.state)}>
                {requestStateLabel(request.state)}
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
          description={() => (
            <span className="font-mono text-sm">
              {live.repo.id} / {request.id}
            </span>
          )}
          title={request.title}
        />

        <RequestFacts request={request} />

        <RequestActions
          activeResolveDisposition={activeResolveDisposition}
          actionError={uiState.actionError}
          commentBody={uiState.commentBody}
          needsResponseBody={uiState.needsResponseBody}
          onCommentBodyChange={(value) =>
            dispatch({ field: 'commentBody', type: 'bodyChanged', value })
          }
          onMergeOpen={() =>
            dispatch({ open: true, type: 'mergeOpenChanged' })
          }
          onNeedsResponseBodyChange={(value) =>
            dispatch({
              field: 'needsResponseBody',
              type: 'bodyChanged',
              value,
            })
          }
          onResolveBodyChange={(value) =>
            dispatch({ field: 'resolveBody', type: 'bodyChanged', value })
          }
          onResolveDispositionChange={(disposition) =>
            dispatch({ disposition, type: 'resolveDispositionChanged' })
          }
          onResponseBodyChange={(value) =>
            dispatch({ field: 'responseBody', type: 'bodyChanged', value })
          }
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

        <RequestTimeline detail={detail} />
      </PageContent>

      <RequestMergeDialog
        error={errorFor(uiState.actionError, 'merge')}
        onConfirm={submitMerge}
        onOpenChange={(open) => dispatch({ open, type: 'mergeOpenChanged' })}
        open={uiState.mergeOpen}
        pending={uiState.pendingAction === 'merge'}
        request={request}
      />
    </main>
  )
}

function requestDetailUiReducer(
  state: RequestDetailUiState,
  action: RequestDetailUiAction,
): RequestDetailUiState {
  switch (action.type) {
    case 'bodyChanged':
      return { ...state, [action.field]: action.value }
    case 'resolveDispositionChanged':
      return { ...state, resolveDisposition: action.disposition }
    case 'mergeOpenChanged':
      return { ...state, mergeOpen: action.open }
    case 'actionStarted':
      return { ...state, actionError: null, pendingAction: action.key }
    case 'actionSucceeded':
      return {
        ...state,
        ...(action.resetField ? { [action.resetField]: '' } : {}),
        actionError: null,
        mergeOpen: action.closeMerge ? false : state.mergeOpen,
        pendingAction: null,
      }
    case 'actionFailed':
      return {
        ...state,
        actionError: { key: action.key, message: action.message },
        pendingAction: null,
      }
  }
}

function RequestFacts({ request }: { request: RequestSummary }) {
  return (
    <SectionRows>
      <SectionRow
        columns="compact"
        description="The branch and target this request is asking maintainers to evaluate."
        icon={<GitBranch className="size-4 text-muted-foreground" />}
        title="Branch"
      >
        <div className="grid gap-2 text-sm leading-5">
          <KeyValue label="Target" value={request.target_branch} />
          <KeyValue label="Request ref" value={request.request_ref} />
          <div className="flex flex-wrap gap-1.5">
            <Badge variant="outline">{requestBaseAudienceLabel(request)}</Badge>
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
  activeResolveDisposition: RequestWorkflowDisposition
  actionError: ActionError | null
  commentBody: string
  needsResponseBody: string
  onCommentBodyChange: (value: string) => void
  onMergeOpen: () => void
  onNeedsResponseBodyChange: (value: string) => void
  onResolveBodyChange: (value: string) => void
  onResolveDispositionChange: (value: RequestWorkflowDisposition) => void
  onResponseBodyChange: (value: string) => void
  onSubmitComment: (event: FormEvent<HTMLFormElement>) => void
  onSubmitNeedsResponse: (event: FormEvent<HTMLFormElement>) => void
  onSubmitResolution: (event: FormEvent<HTMLFormElement>) => void
  onSubmitResponse: (event: FormEvent<HTMLFormElement>) => void
  pendingAction: ActionKey | null
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
                      event.target.value as RequestWorkflowDisposition,
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
                    settlementPreviewFor(
                      request.stake_credits,
                      activeResolveDisposition,
                    ),
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
            <span>{pending ? 'Merging' : 'Merge request'}</span>
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

function RequestTimeline({ detail }: { detail: RequestDetail }) {
  return (
    <section className="mt-8">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-balance text-lg font-semibold leading-7">
          Timeline
        </h2>
        <Badge variant="neutral">{detail.events.length} events</Badge>
      </div>
      <div className="mt-2 divide-y divide-border border-t border-border">
        {detail.events.map((event) => {
          const body = requestEventBody(event)
          return (
            <article className="grid gap-2 py-4" key={event.id}>
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">{eventKindLabel(event.kind)}</Badge>
                <span className="font-mono text-xs tabular-nums text-muted-foreground">
                  {formatUnixDate(event.created_at_unix)}
                </span>
                <span className="truncate font-mono text-xs text-muted-foreground">
                  {event.actor_user_id}
                </span>
              </div>
              {body && (
                <p className="text-pretty whitespace-pre-wrap text-sm leading-6">
                  {body}
                </p>
              )}
            </article>
          )
        })}
      </div>
    </section>
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
          <span>{pending ? pendingLabel : label}</span>
        </Button>
      </div>
      {error && <ActionErrorMessage>{error}</ActionErrorMessage>}
    </div>
  )
}

function ActionErrorMessage({ children }: { children: ReactNode }) {
  return <p className="text-sm leading-5 text-destructive">{children}</p>
}

function KeyValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid gap-1">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      <span className="break-all font-mono text-xs leading-5">{value}</span>
    </div>
  )
}

function errorFor(actionError: ActionError | null, key: ActionKey) {
  return actionError?.key === key ? actionError.message : null
}
