import type {
  CommitDetail,
  CommitFile,
  CommitSummary,
  ProjectionPreviewAudience,
  RequestRevisions,
  ReviewFileDiff,
} from '@/api/types'
import type { LoadRequestRevisionCommitInput } from '@/api/requests'
import { PendingSurface } from '@/components/pending-surface'
import { BlockSkeleton, TextSkeleton } from '@/components/ui/skeleton'
import {
  historyDiffCacheKey,
  peekHistoryDiffCache,
  readHistoryDiffCache,
  writeHistoryDiffCache,
} from '@/features/history/history-resource-cache'
import { HistoryWorkbench } from '@/features/history/history-workbench'
import type {
  CommitDetailState,
  CommitFileDiffState,
} from '@/features/history/history-state'
import { useCachedResource } from '@/lib/use-cached-resource'
import { Link } from '@tanstack/react-router'
import { GitCommit, MessageSquare } from 'lucide-react'
import { useCallback, useMemo, useState } from 'react'
import { compactDiscussionSummary } from './discussion-preview-text'
import type { LoadDiscussionsInput } from './request-discussion-api'
import {
  discussionsForRequestCommit,
  missingRequestCommitFileError,
  orderedRequestCommits,
  requestCommitForListId,
  requestChangeSelection,
  requestRevisionCommitId,
} from './request-changes-model'
import type {
  RequestDiscussion,
  RequestDiscussionPage,
} from './request-discussion-types'

export type RequestChangesSearch = {
  commit?: string
  path?: string
  revision?: string
}

export type RequestChangesDiscussionReferences = {
  all: RequestDiscussionPage | null
  byCommit: Record<string, RequestDiscussionPage | null>
}

type DiscussionReferenceState = {
  discussions: RequestDiscussion[]
  error: string | null
  loadMore?: () => void
  retry?: () => void
  status: 'failed' | 'loaded' | 'loading'
}

export function RequestChangesWorkbench({
  audience,
  initialDiscussionReferences,
  loadDiff,
  loadDiscussions,
  onSearchChange,
  params,
  repoId,
  revisions,
  search,
}: {
  audience: ProjectionPreviewAudience
  initialDiscussionReferences: RequestChangesDiscussionReferences
  loadDiff: (
    input: LoadRequestRevisionCommitInput & { path: string },
    signal?: AbortSignal,
  ) => Promise<ReviewFileDiff>
  loadDiscussions: (input: LoadDiscussionsInput) => Promise<{
    discussions: RequestDiscussion[]
    next_cursor: string | null
  }>
  onSearchChange: (search: RequestChangesSearch) => void
  params: { owner: string; repo: string; request_id: string }
  repoId: string
  revisions: RequestRevisions
  search: RequestChangesSearch
}) {
  const model = useRequestChangesModel({
    audience,
    loadDiff,
    onSearchChange,
    params,
    repoId,
    revisions,
    search,
  })
  const discussionReferences = useRequestDiscussionReferences({
    commitOid: model.selectedCommitOid,
    initialReferences: initialDiscussionReferences,
    loadDiscussions,
    params,
    revision: model.selectedRevision,
  })
  const references = useMemo(
    () => discussionsForRequestCommit(
      discussionReferences.discussions,
      model.selectedRevision,
      model.selectedCommitOid,
    ),
    [discussionReferences.discussions, model.selectedCommitOid, model.selectedRevision],
  )
  const commitContext = useMemo(
    () => model.selectedRevision ? (
      <RequestCommitContext
        commit={model.selectedCommitSummary}
        discussions={references}
        discussionReferences={discussionReferences}
        hasEarlierRevisions={revisions.has_earlier_revisions}
        params={params}
        revision={model.selectedRevision}
      />
    ) : undefined,
    [discussionReferences, model.selectedCommitSummary, model.selectedRevision, params, references, revisions.has_earlier_revisions],
  )
  const emptyDescription = model.commitState.status === 'failed'
    ? model.commitState.error ?? 'Request changes are unavailable.'
    : 'Changes appear here after the request branch is pushed.'
  const emptyTitle = model.commitState.status === 'failed'
    ? 'Request changes unavailable'
    : 'No request changes yet'

  return (
    <HistoryWorkbench
      commitContext={commitContext}
      commitState={model.commitState}
      commits={model.commits}
      diffIdentity={model.diffIdentity}
      emptyDescription={emptyDescription}
      emptyTitle={emptyTitle}
      fileDiffState={model.fileDiffState}
      onCloseDiff={model.closeDiff}
      onRetryCommit={model.retryCommit}
      onRetryDiff={model.retryDiff}
      onSelectCommit={model.selectCommit}
      onSelectFile={model.selectFile}
      selectedCommitId={model.selectedCommitId}
      selectedFilePath={model.selectedFilePath}
    />
  )
}

function useRequestDiscussionReferences({
  commitOid,
  initialReferences,
  loadDiscussions,
  params,
  revision,
}: {
  commitOid: string | null
  initialReferences: RequestChangesDiscussionReferences
  loadDiscussions: (input: LoadDiscussionsInput) => Promise<{
    discussions: RequestDiscussion[]
    next_cursor: string | null
  }>
  params: { owner: string; repo: string; request_id: string }
  revision: RequestRevisions['revisions'][number] | null
}): DiscussionReferenceState {
  const key = revision && commitOid
    ? requestRevisionCommitId(revision.id, commitOid)
    : null
  const initialPage = initialReferences.all
    ?? (key ? initialReferences.byCommit[key] ?? null : null)
  const [resource, setResource] = useState<{
    discussions: RequestDiscussion[]
    error: string | null
    initialPage: RequestDiscussionPage | null
    nextCursor: string | null
    status: 'failed' | 'loaded' | 'loading'
  }>(() => discussionReferenceResource(initialPage))
  const active = resource.initialPage === initialPage
    ? resource
    : discussionReferenceResource(initialPage)
  if (active !== resource) setResource(active)
  const loadPage = useCallback(async () => {
    if (!revision || !commitOid) return
    const cursor = active.nextCursor ?? undefined
    setResource((current) => current.initialPage === initialPage ? {
      ...current,
      error: null,
      status: 'loading',
    } : current)
    try {
      const page = await loadDiscussions({
        ...params,
        commit_oid: commitOid,
        cursor,
        include_revision_anchor: commitOid === revision.commits.at(-1)?.oid,
        limit: 100,
        revision_id: revision.id,
      })
      setResource((current) => current.initialPage === initialPage ? {
        discussions: cursor
          ? [...current.discussions, ...page.discussions]
          : page.discussions,
        error: null,
        initialPage,
        nextCursor: page.next_cursor,
        status: 'loaded',
      } : current)
    } catch (error) {
      setResource((current) => current.initialPage === initialPage ? {
        ...current,
        error: error instanceof Error ? error.message : 'Discussion references are unavailable.',
        status: 'failed',
      } : current)
    }
  }, [active.nextCursor, commitOid, initialPage, loadDiscussions, params, revision])
  return {
    discussions: active.discussions,
    error: active.error,
    loadMore: active.nextCursor && active.status === 'loaded'
      ? () => void loadPage()
      : undefined,
    retry: active.status === 'failed'
      ? () => void loadPage()
      : undefined,
    status: active.status,
  }
}

function discussionReferenceResource(initialPage: RequestDiscussionPage | null) {
  return {
    discussions: initialPage?.discussions ?? [],
    error: initialPage ? null : 'Discussion references are unavailable.',
    initialPage,
    nextCursor: initialPage?.next_cursor ?? null,
    status: initialPage ? 'loaded' as const : 'failed' as const,
  }
}

function useRequestChangesModel({
  audience,
  loadDiff,
  onSearchChange,
  params,
  repoId,
  revisions,
  search,
}: {
  audience: ProjectionPreviewAudience
  loadDiff: (
    input: LoadRequestRevisionCommitInput & { path: string },
    signal?: AbortSignal,
  ) => Promise<ReviewFileDiff>
  onSearchChange: (search: RequestChangesSearch) => void
  params: { owner: string; repo: string; request_id: string }
  repoId: string
  revisions: RequestRevisions
  search: RequestChangesSearch
}) {
  const orderedRevisions = revisions.revisions
  const commits = useMemo(
    () => orderedRequestCommits(orderedRevisions),
    [orderedRevisions],
  )
  const selection = useMemo(
    () => requestChangeSelection(
      orderedRevisions,
      revisions.review_revision_id,
      search,
    ),
    [orderedRevisions, revisions.review_revision_id, search],
  )
  const selectedRevision = selection.revision
  const selectedRevisionId = selectedRevision?.id ?? null
  const selectedCommitOid = selection.commit
  const selectedCommitId = selectedRevision && selectedCommitOid
    ? requestRevisionCommitId(selectedRevision.id, selectedCommitOid)
    : null
  const generation = `${orderedRevisions.at(-1)?.position ?? 0}`
  const viewKey = `request:${params.request_id}:${selectedRevision?.id ?? 'none'}`
  const selectedCommitSummary = selectedRevision?.commits.find(
    ({ oid }) => oid === selectedCommitOid,
  ) ?? null
  const selectedCommit = selectedRevision && selectedCommitSummary
    ? commitDetail(
        selectedRevision.id,
        selectedCommitSummary,
        audience,
        repoId,
        params.request_id,
      )
    : null
  const selectedFilePath = search.path ?? null
  const selectedFile = selectedCommit?.files.find(
    ({ path }) => path === selectedFilePath,
  ) ?? null
  const diffIdentity = selectedCommitId && selectedFile && selectedRevision
    ? historyDiffCacheKey({
        audience,
        commit: selectedCommitId,
        generation,
        newOid: selectedFile.new_oid,
        oldOid: selectedFile.old_oid,
        path: selectedFile.path,
        repoId,
        viewKey,
      })
    : null
  const loadSelectedDiff = useCallback(
    async (signal: AbortSignal) => {
      if (!selectedCommitOid || !selectedRevisionId || !selectedFilePath) {
        throw new Error('Select a changed file.')
      }
      return loadDiff({
        ...params,
        commit_oid: selectedCommitOid,
        path: selectedFilePath,
        revision_id: selectedRevisionId,
      }, signal)
    },
    [loadDiff, params, selectedCommitOid, selectedFilePath, selectedRevisionId],
  )
  const diffResource = useCachedResource({
    fallbackError: 'Request file diff is unavailable.',
    identity: diffIdentity,
    load: loadSelectedDiff,
    peek: peekHistoryDiffCache,
    read: readHistoryDiffCache,
    write: writeHistoryDiffCache,
  })
  const commitState: CommitDetailState = selection.error
    ? { commit: null, error: selection.error, status: 'failed' }
    : selectedCommit
      ? { commit: selectedCommit, error: null, status: 'loaded' }
      : { commit: null, error: null, status: 'idle' }
  const fileDiffState: CommitFileDiffState =
    selectedFilePath && selectedCommit && !selectedFile
      ? {
          diff: null,
          error: selectedCommitSummary
            ? missingRequestCommitFileError(selectedCommitSummary)
            : 'This file is not part of the selected commit.',
          status: 'failed',
        }
      : resourceToDiffState(diffResource)

  function replaceSelection(
    revision: typeof selectedRevision,
    commitOid: string | null,
    path: string | null,
  ) {
    onSearchChange({
      commit: commitOid ?? undefined,
      path: path ?? undefined,
      revision: revision?.id,
    })
  }

  return {
    closeDiff: () => replaceSelection(selectedRevision, selectedCommitOid, null),
    commits,
    commitState,
    diffIdentity,
    fileDiffState,
    retryCommit: undefined,
    retryDiff: selectedFilePath && selectedCommit && !selectedFile
      ? undefined
      : diffResource.retry,
    selectCommit: (commit: CommitSummary) => {
      const selected = requestCommitForListId(orderedRevisions, commit.projected_id)
      replaceSelection(selected?.revision ?? null, selected?.commitOid ?? null, null)
    },
    selectFile: (file: CommitFile) =>
      replaceSelection(selectedRevision, selectedCommitOid, file.path),
    selectedCommitId,
    selectedCommitOid,
    selectedCommitSummary,
    selectedFilePath,
    selectedRevision,
  }
}

function RequestCommitContext({
  commit,
  discussionReferences,
  discussions,
  hasEarlierRevisions,
  params,
  revision,
}: {
  commit: RequestRevisions['revisions'][number]['commits'][number] | null
  discussionReferences: DiscussionReferenceState
  discussions: RequestDiscussion[]
  hasEarlierRevisions: boolean
  params: { owner: string; repo: string; request_id: string }
  revision: RequestRevisions['revisions'][number]
}) {
  return (
    <div className="mt-3 border-t border-border pt-3 text-xs text-muted-foreground">
      <div className="flex flex-wrap items-center gap-2">
        <GitCommit className="size-3.5" />
        <span>Revision {revision.position} by {revision.actor.handle}</span>
        {revision.old_head_oid && revision.new_head_oid ? (
          <span className="font-mono">
            {shortOid(revision.old_head_oid)} → {shortOid(revision.new_head_oid)}
          </span>
        ) : null}
      </div>
      {revision.inspection !== 'Complete' || hasEarlierRevisions ? (
        <p className="mt-2">
          {[
            revision.inspection === 'Incomplete'
              ? 'Commit inspection for this revision is incomplete.'
              : null,
            revision.inspection === 'Unavailable'
              ? 'Commit inspection for this revision is unavailable.'
              : null,
            hasEarlierRevisions ? 'Earlier request revisions are omitted.' : null,
          ].filter(Boolean).join(' ')}
        </p>
      ) : null}
      {commit?.files_truncated ? (
        <p className="mt-2">
          Showing {commit.files.length} of {commit.change_count} changed files because the file list is bounded.
        </p>
      ) : null}
      {discussions.length > 0 ? (
        <div
          aria-label="Related discussions"
          className="scope-content-enter mt-3 space-y-2"
        >
          {discussions.map((discussion) => (
            <Link
              className="flex min-w-0 items-center gap-2 text-foreground hover:text-brand"
              hash={`discussion-${discussion.id}`}
              key={discussion.id}
              params={{
                owner: params.owner,
                repo: params.repo,
                requestId: params.request_id,
              }}
              search={{ discussion: discussion.id }}
              to="/$owner/$repo/requests/$requestId"
            >
              <MessageSquare className="size-3.5 shrink-0" />
              <span className="truncate">{compactDiscussionSummary(discussion.body_markdown)}</span>
              <span className="ml-auto shrink-0 text-muted-foreground">
                {discussion.status === 'Resolved' ? 'Resolved' : 'Open'}
              </span>
            </Link>
          ))}
        </div>
      ) : null}
      {discussionReferences.status === 'loading' ? (
        <PendingSurface
          className="mt-3 min-h-6"
          delay
          label="Loading discussion references"
        >
          <div className="space-y-2">
            <div className="flex items-center gap-2">
              <BlockSkeleton className="size-3.5" />
              <TextSkeleton length="long" size="meta" />
              <TextSkeleton className="ml-auto" length="tiny" size="meta" />
            </div>
            <div className="flex items-center gap-2">
              <BlockSkeleton className="size-3.5" />
              <TextSkeleton length="medium" size="meta" />
              <TextSkeleton className="ml-auto" length="tiny" size="meta" />
            </div>
          </div>
        </PendingSurface>
      ) : null}
      {discussionReferences.status === 'failed' ? (
        <div className="mt-3 flex items-center gap-2">
          <span>{discussionReferences.error}</span>
          <button
            className="font-medium text-foreground hover:text-brand"
            onClick={discussionReferences.retry}
            type="button"
          >
            Retry
          </button>
        </div>
      ) : null}
      {discussionReferences.loadMore ? (
        <button
          className="mt-3 font-medium text-foreground hover:text-brand"
          onClick={discussionReferences.loadMore}
          type="button"
        >
          Load more discussion references
        </button>
      ) : null}
    </div>
  )
}

function commitDetail(
  revisionId: string,
  commit: RequestRevisions['revisions'][number]['commits'][number],
  audience: ProjectionPreviewAudience,
  repoId: string,
  requestId: string,
): CommitDetail {
  return {
    audience,
    author: commit.author,
    change_count: commit.change_count,
    files_truncated: commit.files_truncated,
    files: commit.files.map((file) => ({
      ...file,
      path: `/${file.path.replace(/^\/+/, '')}`,
    })),
    logical_commit_id: commit.oid,
    message: commit.message,
    parent_projected_id: commit.parent_oids[0] ?? null,
    projected_id: requestRevisionCommitId(revisionId, commit.oid),
    repo_id: repoId,
    view_key: `request:${requestId}:${revisionId}`,
  }
}

function resourceToDiffState(
  resource: ReturnType<typeof useCachedResource<ReviewFileDiff>>,
): CommitFileDiffState {
  if (resource.status === 'loaded') {
    return { diff: resource.value, error: null, status: 'loaded' }
  }
  if (resource.status === 'failed') {
    return { diff: null, error: resource.error, status: 'failed' }
  }
  return { diff: null, error: null, status: resource.status }
}

function shortOid(oid: string) {
  return oid.slice(0, 8)
}
