import type {
  CommitFile,
  CommitHistory,
  CommitSummary,
  ProjectionPreviewAudience,
  RepoParams,
} from '@/api/types'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { AudienceToggle } from '@/features/history/history-audience-toggle'
import { HistoryWorkbench } from '@/features/history/history-workbench'
import {
  historyCommitCacheKey,
  historyDiffCacheKey,
  peekHistoryCommitCache,
  peekHistoryDiffCache,
  readHistoryCommitCache,
  readHistoryDiffCache,
  writeHistoryCommitCache,
  writeHistoryDiffCache,
} from '@/features/history/history-resource-cache'
import {
  resourceToCommitState,
  resourceToDiffState,
  type CommitDetailState,
  type CommitFileDiffState,
} from '@/features/history/history-state'
import { useCachedResource } from '@/lib/use-cached-resource'
import {
  loadCommitDetail,
  loadCommitFileDiff,
} from '@/routes/-repo-history-actions'
import { useNavigate } from '@tanstack/react-router'
import { useCallback, useMemo } from 'react'
import { changeCountLabel } from '../review/review-labels'

export type CommitHistories = {
  private: CommitHistory | null
  public: CommitHistory | null
}

type HistoryPageProps = {
  histories: CommitHistories
  params: RepoParams
  search: {
    audience?: ProjectionPreviewAudience
    commit?: string
    path?: string
  }
}

export function HistoryPage(props: HistoryPageProps) {
  const {
    audience,
    availableAudiences,
    closeDiff,
    commitState,
    commits,
    diffIdentity,
    fileDiffState,
    retryCommit,
    retryDiff,
    selectAudience,
    selectCommit,
    selectFile,
    selectedCommit,
    selectedCommitId,
    selectedFilePath,
  } = useHistoryPageModel(props)

  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={availableAudiences.length > 1 ? (
          <AudienceToggle
            audience={audience}
            availableAudiences={availableAudiences}
            onSelect={selectAudience}
          />
        ) : undefined}
        summary={`${commits.length} ${commits.length === 1 ? 'commit' : 'commits'}${selectedCommit ? ` · ${changeCountLabel(selectedCommit.change_count)}` : ''}`}
        title="History"
      />
      <HistoryWorkbench
        commitState={commitState}
        commits={commits}
        diffIdentity={diffIdentity}
        emptyDescription="History appears here once Scope has applied commits."
        emptyTitle="No commits yet"
        fileDiffState={fileDiffState}
        onCloseDiff={closeDiff}
        onRetryCommit={retryCommit}
        onRetryDiff={retryDiff}
        onSelectCommit={selectCommit}
        onSelectFile={selectFile}
        selectedCommitId={selectedCommitId}
        selectedFilePath={selectedFilePath}
      />
    </WorkbenchPane>
  )
}

function useHistoryPageModel({ histories, params, search }: HistoryPageProps) {
  const navigate = useNavigate()
  const availableAudiences = useMemo(
    () =>
      (['private', 'public'] as const).filter(
        (option) => histories[option] !== null,
      ),
    [histories],
  )
  const audience = selectedAudience(histories, search.audience)
  const history = histories[audience] ?? histories.public ?? histories.private
  const baseCommits = useMemo(
    () => [...(history?.commits ?? [])].reverse(),
    [history?.commits],
  )
  const requestedCommitUnavailable = Boolean(
    search.commit && history && !history.commits.some(
      (commit) => commit.projected_id === search.commit,
    ),
  )
  const selectedCommitId = requestedCommitUnavailable
    ? null
    : search.commit ?? latestCommitId(history)
  const repoId = history?.repo_id ?? `${params.owner}/${params.repo}`
  const commitIdentity = selectedCommitId && history
    ? historyCommitCacheKey({
        audience,
        commit: selectedCommitId,
        generation: history.generation,
        repoId: history.repo_id,
        viewKey: history.view_key,
      })
    : null
  const loadSelectedCommit = useCallback(
    async (signal: AbortSignal) => {
      return loadCommitDetail({
        data: {
          audience,
          commit: selectedCommitId ?? '',
          owner: params.owner,
          repo: params.repo,
        },
        signal,
      })
    },
    [audience, params, selectedCommitId],
  )
  const commitResource = useCachedResource({
    fallbackError: 'Resource is unavailable.',
    identity: commitIdentity,
    load: loadSelectedCommit,
    peek: peekHistoryCommitCache,
    read: readHistoryCommitCache,
    write: writeHistoryCommitCache,
  })
  const selectedCommit = commitResource.value
  const commits = baseCommits
  const selectedFilePath = search.path ?? null
  const selectedFile = selectedCommit?.files.find(
    (file) => file.path === selectedFilePath,
  ) ?? null
  const diffIdentity = selectedCommitId && selectedFile && history
    ? historyDiffCacheKey({
        audience,
        commit: selectedCommitId,
        generation: history.generation,
        newOid: selectedFile.new_oid,
        oldOid: selectedFile.old_oid,
        path: selectedFile.path,
        repoId,
        viewKey: history.view_key,
      })
    : null
  const loadSelectedDiff = useCallback(
    (signal: AbortSignal) => loadCommitFileDiff({
      data: {
        audience,
        commit: selectedCommitId ?? '',
        owner: params.owner,
        path: selectedFilePath ?? '',
        repo: params.repo,
      },
      signal,
    }),
    [audience, params, selectedCommitId, selectedFilePath],
  )
  const diffResource = useCachedResource({
    fallbackError: 'Resource is unavailable.',
    identity: diffIdentity,
    load: loadSelectedDiff,
    peek: peekHistoryDiffCache,
    read: readHistoryDiffCache,
    write: writeHistoryDiffCache,
  })
  const commitState: CommitDetailState = requestedCommitUnavailable
    ? { commit: null, error: 'The requested commit is not available in this history view.', status: 'failed' }
    : resourceToCommitState(commitResource)
  const fileDiffState: CommitFileDiffState =
    selectedFilePath && selectedCommit && !selectedFile
      ? { diff: null, error: 'This file is not part of the selected commit.', status: 'failed' }
      : resourceToDiffState(diffResource)

  function replaceHistorySearch(
    nextAudience: ProjectionPreviewAudience,
    nextCommitId: string | null,
    nextPath: string | null = null,
  ) {
    void navigate({
      params,
      replace: true,
      resetScroll: false,
      search: {
        audience: nextAudience,
        commit: nextCommitId ?? undefined,
        path: nextPath ?? undefined,
      },
      to: '/$owner/$repo/history',
    })
  }

  return {
    audience,
    availableAudiences,
    closeDiff: () => replaceHistorySearch(audience, selectedCommitId),
    commitState,
    commits,
    diffIdentity,
    fileDiffState,
    retryCommit: requestedCommitUnavailable ? undefined : commitResource.retry,
    retryDiff: selectedFilePath && selectedCommit && !selectedFile
      ? undefined
      : diffResource.retry,
    selectAudience: (nextAudience: ProjectionPreviewAudience) => {
      const nextHistory = histories[nextAudience]
      if (nextHistory) {
        replaceHistorySearch(nextAudience, latestCommitId(nextHistory))
      }
    },
    selectCommit: (commit: CommitSummary) =>
      replaceHistorySearch(audience, commit.projected_id),
    selectFile: (file: CommitFile) =>
      replaceHistorySearch(audience, selectedCommitId, file.path),
    selectedCommit,
    selectedCommitId,
    selectedFilePath,
  }
}

function selectedAudience(
  histories: CommitHistories,
  requestedAudience?: ProjectionPreviewAudience,
): ProjectionPreviewAudience {
  if (requestedAudience && histories[requestedAudience]) {
    return requestedAudience
  }
  return histories.private ? 'private' : 'public'
}

function latestCommitId(history: CommitHistory | null) {
  return history?.commits.at(-1)?.projected_id ?? null
}
