import type {
  User,
  RepoLiveState,
  RepoParams,
  RequestChanges,
  RequestMutation,
  ReviewFileDiff,
} from '@/api/types'
import { DestructiveActionDialog } from '@/components/destructive-action-dialog'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { RepoShell } from '@/components/repo-shell'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { WorkbenchHeader } from '@/components/workbench-header'
import { Link } from '@tanstack/react-router'
import { Coins, ShieldQuestion, Trash2 } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { RequestActivity } from './request-activity'
import { RequestChangesSection } from './request-changes-section'
import { RequestContextRail } from './request-context-rail'
import type {
  CreateDiscussionInput,
  CreateReplyInput,
  LoadActivityInput,
  LoadDiscussionsInput,
  LoadRepliesInput,
  MarkDiscussionReadInput,
  RequestDiscussionActionInput,
  RequestDiscussionRepliesPage,
  UpdateDescriptionInput,
} from './request-discussion-api'
import { RequestDiscussionWorkbench } from './request-discussion-workbench'
import type {
  RequestDiscussionActions,
} from './request-discussion-store'
import type {
  RequestActivityPage,
  RequestDiscussion,
  RequestDiscussionChanges,
  RequestDiscussionFilter,
  RequestDiscussionMutation,
  RequestDiscussionPage,
  RequestDiscussionReplyMutation,
  RequestDiscussionSort,
} from './request-discussion-types'
import {
  RequestReviewNavigation,
  type RequestReviewView,
} from './request-review-navigation'
import { RequestMergeDialog } from './request-merge-dialog'
import {
  requestMergeabilityLabel,
  requestMergeabilityTone,
  requestStatusLabel,
  requestStatusTone,
} from './request-labels'
import {
  type RequestActionsProps,
  useRequestActions,
} from './use-request-actions'

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
            <ShieldQuestion className="mt-0.5 size-4 shrink-0" />
            <p>
              Private request branches are available only to repository
              maintainers. Sign in with an account that has access, or return
              to the request list.
            </p>
          </div>
        </section>
      </PageContent>
    </RepoShell>
  )
}

type RequestDetailPageProps = RequestActionsProps & {
  actor: User | null
  activity: RequestActivityPage | null
  changes: RequestChanges | null
  changesError: string | null
  createDiscussion: (
    input: CreateDiscussionInput,
  ) => Promise<RequestDiscussionMutation>
  createReply: (
    input: CreateReplyInput,
  ) => Promise<RequestDiscussionReplyMutation>
  discussionFilter: RequestDiscussionFilter
  discussionPage: RequestDiscussionPage | null
  discussionSort: RequestDiscussionSort
  live: RepoLiveState
  loadActivity: (
    input: LoadActivityInput,
  ) => Promise<RequestActivityPage>
  loadDiscussions: (
    input: LoadDiscussionsInput,
  ) => Promise<RequestDiscussionPage>
  loadDiscussionChanges: (
    input: {
      after: number
      owner: string
      repo: string
      request_id: string
    },
  ) => Promise<RequestDiscussionChanges>
  loadReplies: (
    input: LoadRepliesInput,
  ) => Promise<RequestDiscussionRepliesPage>
  markDiscussionRead: (input: MarkDiscussionReadInput) => Promise<unknown>
  onDiscussionQueryChange: (query: {
    filter: RequestDiscussionFilter
    sort: RequestDiscussionSort
  }) => void
  onSelectFile: (path: string) => void
  onViewChange: (view: RequestReviewView) => void
  reopenAndReply: (
    input: CreateReplyInput,
  ) => Promise<RequestDiscussionReplyMutation>
  reopenDiscussion: (
    input: RequestDiscussionActionInput,
  ) => Promise<RequestDiscussionMutation>
  resolveDiscussion: (
    input: RequestDiscussionActionInput,
  ) => Promise<RequestDiscussionMutation>
  selectedDiff: ReviewFileDiff | null
  selectedDiffError: string | null
  selectedPath: string | null
  updateDescription: (input: UpdateDescriptionInput) => Promise<RequestMutation>
  view: RequestReviewView
}

export function RequestDetailPage(props: RequestDetailPageProps) {
  const {
    activity,
    actor: accountActor,
    changes,
    changesError,
    createDiscussion,
    createReply,
    detail,
    discussionFilter,
    discussionPage,
    discussionSort,
    live,
    loadActivity,
    loadDiscussions,
    loadDiscussionChanges,
    loadReplies,
    markDiscussionRead,
    onDiscussionQueryChange,
    onSelectFile,
    onViewChange,
    params,
    reopenAndReply,
    reopenDiscussion,
    resolveDiscussion,
    selectedDiff,
    selectedDiffError,
    selectedPath,
    updateDescription,
    view,
  } = props
  const controller = useRequestActions(props)
  const { request } = controller
  const serverDescription = request.description_markdown
  const [descriptionOverride, setDescriptionOverride] = useState<{
    server: string
    value: string
  } | null>(null)
  const description =
    descriptionOverride?.server === serverDescription
      ? descriptionOverride.value
      : serverDescription
  const actor = useMemo(
    () => ({
      handle: accountActor?.handle ?? 'Anonymous',
      id: accountActor?.id ?? 'anonymous',
    }),
    [accountActor?.handle, accountActor?.id],
  )
  const discussionParams = useMemo(
    () => ({
      owner: params.owner,
      repo: params.repo,
      request_id: request.id,
    }),
    [params.owner, params.repo, request.id],
  )
  const discussionActions: RequestDiscussionActions = useMemo(
    () => ({
      create: createDiscussion,
      load: loadDiscussions,
      loadChanges: loadDiscussionChanges,
      markRead: markDiscussionRead,
      reopen: reopenDiscussion,
      resolve: resolveDiscussion,
    }),
    [
      createDiscussion,
      loadDiscussionChanges,
      loadDiscussions,
      markDiscussionRead,
      reopenDiscussion,
      resolveDiscussion,
    ],
  )
  const threadActions = useMemo(
    () => ({ createReply, loadReplies, reopenAndReply }),
    [createReply, loadReplies, reopenAndReply],
  )
  const isMaintainer = live.repo.access.actor !== 'Public'
  const canResolveDiscussion = useCallback(
    (discussion: RequestDiscussion) =>
      request.state !== 'Resolved' &&
      request.state !== 'Withdrawn' &&
      (
        isMaintainer ||
        actor.id === discussion.author.id ||
        actor.id === request.author_user_id
      ),
    [actor.id, isMaintainer, request.author_user_id, request.state],
  )

  async function saveDescription(nextDescription: string) {
    try {
      await updateDescription({
        ...discussionParams,
        description_markdown: nextDescription,
      })
      setDescriptionOverride({
        server: serverDescription,
        value: nextDescription,
      })
      return true
    } catch {
      return false
    }
  }

  return (
    <RepoShell params={params}>
      <WorkbenchHeader
        actions={(
          <div className="flex flex-wrap items-center gap-2">
            {request.permissions.can_delete ? (
              <Button
                disabled={controller.uiState.pendingAction === 'delete'}
                onClick={() => controller.setDeleteOpen(true)}
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
        count={(
          <span className="font-mono">
            {request.name} / {request.id}
          </span>
        )}
        description={(
          <div className="flex flex-wrap items-center gap-2">
            <LifecycleBadge state={live.repo.lifecycle_state} />
            <Badge variant={requestStatusTone(request)}>
              {requestStatusLabel(request)}
            </Badge>
            <Badge variant={requestMergeabilityTone(request)}>
              {requestMergeabilityLabel(request)}
            </Badge>
            <Badge variant="neutral">
              <Coins className="size-3" />
              {request.stake_credits}
            </Badge>
          </div>
        )}
        eyebrow="Request"
        title={request.title}
      />
      <div className="px-5 lg:px-7">
        <RequestReviewNavigation onChange={onViewChange} view={view} />
      </div>

      {view === 'discussion' && discussionPage ? (
        <RequestDiscussionWorkbench
          actions={discussionActions}
          actor={actor}
          canResolve={canResolveDiscussion}
          contextRail={(
            <RequestContextRail
              activeResolveDisposition={controller.activeResolveDisposition}
              actionError={controller.uiState.actionError}
              needsResponseBody={controller.uiState.needsResponseBody}
              onMergeOpen={() => controller.setMergeOpen(true)}
              onNeedsResponseBodyChange={controller.setNeedsResponseBody}
              onResolveBodyChange={controller.setResolveBody}
              onResolveDispositionChange={controller.setResolveDisposition}
              onResponseBodyChange={controller.setResponseBody}
              onSubmitNeedsResponse={controller.submitNeedsResponse}
              onSubmitResolution={controller.submitResolution}
              onSubmitResponse={controller.submitResponse}
              pendingAction={controller.uiState.pendingAction}
              request={request}
              resolutionOptions={controller.resolutionOptions}
              resolveBody={controller.uiState.resolveBody}
              responseBody={controller.uiState.responseBody}
            />
          )}
          description={description}
          filter={discussionFilter}
          initialPage={discussionPage}
          onDescriptionSave={saveDescription}
          onQueryChange={onDiscussionQueryChange}
          params={discussionParams}
          permissions={{
            canEditDescription:
              request.permissions.can_edit_description,
            canOpenDiscussion: request.permissions.can_open_discussion,
            canReply: request.permissions.can_reply_to_discussion,
          }}
          repoId={live.repo.id}
          request={request}
          sort={discussionSort}
          threadActions={threadActions}
        />
      ) : null}

      {view === 'changes' ? (
        <div className="px-5 pb-8 lg:px-7">
          <RequestChangesSection
            changes={changes}
            error={changesError}
            onSelectFile={onSelectFile}
            selectedDiff={selectedDiff}
            selectedDiffError={selectedDiffError}
            selectedPath={selectedPath}
          />
        </div>
      ) : null}

      {view === 'activity' && activity ? (
        <RequestActivity
          activity={activity}
          loadAfter={(after) =>
            loadActivity({ ...discussionParams, after })
          }
          requestId={request.id}
        />
      ) : null}

      <RequestMergeDialog
        error={
          controller.uiState.actionError?.key === 'merge'
            ? controller.uiState.actionError.message
            : null
        }
        onConfirm={controller.submitMerge}
        onOpenChange={controller.setMergeOpen}
        open={controller.uiState.mergeOpen}
        pending={controller.uiState.pendingAction === 'merge'}
        request={request}
      />
      <DestructiveActionDialog
        confirmLabel={
          request.state === 'Working' ? 'Delete request' : 'Withdraw request'
        }
        description={
          request.state === 'Working'
            ? 'This removes the request branch from Scope. Local Git branches are not deleted.'
            : 'This closes the request and removes it from maintainer review. Its activity remains visible.'
        }
        onConfirm={() => void controller.submitDelete()}
        onOpenChange={(open) => {
          if (controller.uiState.pendingAction !== 'delete') {
            controller.setDeleteOpen(open)
          }
        }}
        open={controller.deleteOpen}
        pending={controller.uiState.pendingAction === 'delete'}
        subject={request.title}
        title={
          request.state === 'Working'
            ? 'Delete working request?'
            : 'Withdraw request?'
        }
      />
    </RepoShell>
  )
}
