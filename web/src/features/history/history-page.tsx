import type {
  CommitFile,
  CommitHistory,
  CommitSummary,
  ProjectionPreviewAudience,
  RepoParams,
} from '@/api/types'
import { EmptyState } from '@/components/empty-state'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { RouteErrorContent } from '@/components/route-error-page'
import { useWorkspaceTabs } from '@/components/use-workspace-tabs'
import type { WorkspaceTabItem } from '@/components/workspace-tab-model'
import { AudienceToggle } from '@/features/history/history-audience-toggle'
import { HistoryWorkbench } from '@/features/history/history-workbench'
import {
  historyCommitCacheKey,
  historyDiffCacheKey,
  peekHistoryCommitCache,
  peekHistoryDiffCache,
  readHistoryCommitCache,
  readHistoryDiffCache,
  readHistoryDiffScroll,
  writeHistoryCommitCache,
  writeHistoryDiffCache,
  writeHistoryDiffScroll,
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
import { History } from 'lucide-react'
import { type ReactNode, useCallback, useMemo } from 'react'
import { changeCountLabel } from '../review/review-labels'

const HISTORY_TAB_SET_ID = 'history-file-diffs'

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
  const { params } = props
  const {
    audience,
    activateFileTab,
    availableAudiences,
    closeFileTab,
    commitState,
    commits,
    diffIdentity,
    fileDiffState,
    fileTabs,
    repoId,
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
      />
      <HistoryWorkbench
        commitState={commitState}
        commits={commits}
        diffIdentity={diffIdentity}
        emptyDescription="History appears here once Scope has applied commits."
        emptyTitle="No commits yet"
        fileDiffState={fileDiffState}
        fileTabs={fileTabs}
        onActivateFileTab={activateFileTab}
        onCloseFileTab={closeFileTab}
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
  const fileTabItems = useMemo(
    () =>
      (selectedCommit?.files ?? []).map((file) => ({
        id: file.path,
        label: fileName(file.path),
        title: file.path.replace(/^\/+/, ''),
      })),
    [selectedCommit?.files],
  )
  const fileTabs = useWorkspaceTabs({
    activeId: selectedFilePath,
    items: fileTabItems,
    storageKey: `history:${repoId}:${audience}:${selectedCommitId ?? 'none'}`,
  })

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
    activateFileTab: (path: string) =>
      replaceHistorySearch(audience, selectedCommitId, path),
    audience,
    availableAudiences,
    closeFileTab: (path: string) => {
      if (!fileTabs.tabs.some((tab) => tab.id === path)) {
        replaceHistorySearch(audience, selectedCommitId)
        return null
      }
      const result = fileTabs.close(path)
      if (path === selectedFilePath) {
        replaceHistorySearch(audience, selectedCommitId, result.activeId)
      }
      return result.focusId
    },
    commitState,
    commits,
    diffIdentity,
    fileDiffState,
    fileTabs: fileTabs.tabs,
    repoId,
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
    selectFile: (file: CommitFile) => {
      fileTabs.prepareOpen(file.path)
      replaceHistorySearch(audience, selectedCommitId, file.path)
    },
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

function fileName(path: string) {
  const displayPath = path.replace(/^\/+/, '')
  return displayPath.split('/').at(-1) ?? displayPath
}
