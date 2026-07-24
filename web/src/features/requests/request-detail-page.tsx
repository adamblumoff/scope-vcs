import type {
  RequestChangeBlockFiles,
  RequestDetail,
  User,
  RepoLiveState,
  RepoParams,
  RequestMutation,
} from '@/api/types'
import type { LoadRequestChangeBlockFilesInput } from '@/api/requests'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { RepoShell } from '@/components/repo-shell'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { WorkbenchHeader } from '@/components/workbench-header'
import { Link } from '@tanstack/react-router'
import { Coins, ShieldQuestion } from 'lucide-react'
import type { ReactNode } from 'react'
import { useCallback, useMemo, useState } from 'react'
import { RequestActivityDrawer } from './request-activity-drawer'
import { RequestContextRail } from './request-context-rail'
import type {
  CreateDiscussionInput,
  CreateReplyInput,
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
  RequestDiscussionMutation,
  RequestDiscussionPage,
  RequestDiscussionReplyMutation,
} from './request-discussion-types'
import { RequestOverflowMenu } from './request-overflow-menu'
import {
  requestMergeabilityLabel,
  requestMergeabilityTone,
  requestStatusLabel,
  requestStatusTone,
} from './request-labels'
import { useRequestActivityHistory } from './use-request-activity-history'

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

type RequestDetailPageProps = {
  detail: RequestDetail
  params: RepoParams
  actor: User | null
  createDiscussion: (
    input: CreateDiscussionInput,
  ) => Promise<RequestDiscussionMutation>
  createReply: (
    input: CreateReplyInput,
  ) => Promise<RequestDiscussionReplyMutation>
  discussionPage: RequestDiscussionPage
  live: RepoLiveState
  loadActivity: () => Promise<RequestActivityPage>
  loadChangeBlockFiles: (
    input: LoadRequestChangeBlockFilesInput,
  ) => Promise<RequestChangeBlockFiles>
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
  reopenAndReply: (
    input: CreateReplyInput,
  ) => Promise<RequestDiscussionReplyMutation>
  reopenDiscussion: (
    input: RequestDiscussionActionInput,
  ) => Promise<RequestDiscussionMutation>
  resolveDiscussion: (
    input: RequestDiscussionActionInput,
  ) => Promise<RequestDiscussionMutation>
  updateDescription: (input: UpdateDescriptionInput) => Promise<RequestMutation>
}

export function RequestDetailPage(props: RequestDetailPageProps) {
  const {
    actor: accountActor,
    createDiscussion,
    createReply,
    detail,
    discussionPage,
    live,
    loadActivity,
    loadChangeBlockFiles,
    loadDiscussions,
    loadDiscussionChanges,
    loadReplies,
    markDiscussionRead,
    params,
    reopenAndReply,
    reopenDiscussion,
    resolveDiscussion,
    updateDescription,
  } = props
  const { request } = detail
  const serverDescription = request.description_markdown
  const history = useRequestActivityHistory(loadActivity)
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
      request.state !== 'Completed' &&
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

  function requestHeader(discussionControls?: ReactNode) {
    return (
      <WorkbenchHeader
        actions={(
          <div className="flex flex-wrap items-center justify-end gap-2">
            {discussionControls}
            <Button
              asChild
              className="h-9"
              size="sm"
              variant="secondary"
            >
              <Link params={params} to="/repos/$owner/$repo/requests">
                Requests
              </Link>
            </Button>
            <RequestOverflowMenu
              onViewHistory={history.openHistory}
            />
          </div>
        )}
        className="sm:flex-col sm:items-stretch xl:flex-row xl:items-end"
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
              {request.current_stake_credits}
            </Badge>
          </div>
        )}
        eyebrow="Request"
        title={request.title}
      />
    )
  }

  return (
    <RepoShell params={params}>
      <RequestDiscussionWorkbench
          actions={discussionActions}
          actor={actor}
          canResolve={canResolveDiscussion}
          contextRail={(
            <RequestContextRail request={request} />
          )}
          description={description}
          header={requestHeader}
          initialPage={discussionPage}
          loadChangeBlockFiles={loadChangeBlockFiles}
          onDescriptionSave={saveDescription}
          params={discussionParams}
          permissions={{
            canEditDescription:
              request.permissions.can_edit_identity,
            canOpenDiscussion: request.permissions.can_open_discussion,
            canReply: request.permissions.can_reply_to_discussion,
          }}
          repoId={live.repo.id}
          request={request}
          threadActions={threadActions}
      />

      <RequestActivityDrawer
        activity={history.activity}
        error={history.error}
        load={history.retry}
        loading={history.loading}
        onOpenChange={history.onOpenChange}
        open={history.open}
      />
    </RepoShell>
  )
}
