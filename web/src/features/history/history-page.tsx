import type {
  CommitDetail,
  CommitFile,
  HistoryEntryDetail,
  HistoryEntrySummary,
  HistoryPage as HistoryPageResponse,
  ProjectionPreviewAudience,
  RepoParams,
} from '@/api/types'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { AudienceToggle } from '@/features/history/history-audience-toggle'
import { CommitDetailPanel } from '@/features/history/history-commit-detail'
import { HistoryEntryList } from '@/features/history/history-entry-list'
import {
  appendHistoryPage,
  historySummary,
} from '@/features/history/history-pagination'
import {
  historyEntryCacheKey,
  historyEntryDiffCacheKey,
  peekHistoryDiffCache,
  peekHistoryEntryCache,
  readHistoryDiffCache,
  readHistoryDiffScroll,
  readHistoryEntryCache,
  writeHistoryDiffCache,
  writeHistoryDiffScroll,
  writeHistoryEntryCache,
} from '@/features/history/history-resource-cache'
import {
  resourceToDiffState,
  type CommitDetailState,
  type CommitFileDiffState,
} from '@/features/history/history-state'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { useCachedResource, type CachedResource } from '@/lib/use-cached-resource'
import {
  loadHistoryEntry,
  loadHistoryEntryFileDiff,
  loadHistoryPage,
} from '@/routes/-repo-history-actions'
import { useNavigate } from '@tanstack/react-router'
import { useCallback, useState } from 'react'

type HistoryPageProps = {
  initialPage: HistoryPageResponse
  params: RepoParams
  search: {
    audience?: ProjectionPreviewAudience
    entry?: string
    path?: string
  }
}

export function HistoryPage(props: HistoryPageProps) {
  const {
    audience,
    availableAudiences,
    closeDiff,
    detailState,
    entries,
    fileDiffState,
    loadOlder,
    loadOlderError,
    loadingOlder,
    retryDetail,
    retryDiff,
    selectAudience,
    selectEntry,
    selectFile,
    selectedDetail,
    selectedEntryId,
    selectedFilePath,
    showLoadOlder,
    diffIdentity,
    saveDiffScroll,
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
        summary={`${historySummary(entries, showLoadOlder)}${selectedDetail ? ` · ${historyDetailCountLabel(selectedDetail)}` : ''}`}
        title="History"
      />
      <section className="border-t border-border">
        {entries.length === 0 && !selectedEntryId ? (
          <div className="px-5 py-12 text-center sm:px-6">
            <h2 className="text-sm font-semibold">No updates yet</h2>
            <p className="mt-1 text-sm text-muted-foreground">
              History appears here after Scope applies an update.
            </p>
          </div>
        ) : (
          <div className="grid grid-cols-1 lg:grid-cols-[minmax(260px,0.4fr)_minmax(0,1.6fr)]">
            <HistoryEntryList
              entries={entries}
              loadOlderError={loadOlderError}
              loadingOlder={loadingOlder}
              onLoadOlder={loadOlder}
              onSelectEntry={selectEntry}
              selectedEntryId={selectedEntryId}
              showLoadOlder={showLoadOlder}
            />
            <div className="min-w-0">
              <CommitDetailPanel
                commitState={detailState}
                diffIdentity={diffIdentity}
                diffScrollTop={readHistoryDiffScroll(diffIdentity)}
                fileDiffState={fileDiffState}
                onCloseDiff={closeDiff}
                onDiffScroll={saveDiffScroll}
                onRetryCommit={retryDetail}
                onRetryDiff={retryDiff}
                onSelectFile={selectFile}
                selectedFilePath={selectedFilePath}
                terminology="update"
                visibilityChanges={selectedDetail?.visibility_changes}
              />
            </div>
          </div>
        )}
      </section>
    </WorkbenchPane>
  )
}

function useHistoryPageModel({ initialPage, params, search }: HistoryPageProps) {
  const navigate = useNavigate()
  const { repo } = useRepoLayout()
  const [loaded, setLoaded] = useState(() => ({
    entries: initialPage.entries,
    next_cursor: initialPage.next_cursor,
  }))
  const [loadingOlder, setLoadingOlder] = useState(false)
  const [loadOlderError, setLoadOlderError] = useState<string | null>(null)
  const audience = initialPage.audience
  const availableAudiences: ProjectionPreviewAudience[] = repo.access.can_read_private_files
    ? ['private', 'public']
    : ['public']
  const selectedEntryId = search.entry ?? loaded.entries[0]?.source_id ?? null
  const entryIdentity = selectedEntryId
    ? historyEntryCacheKey({
        audience,
        entry: selectedEntryId,
        generation: initialPage.generation,
        repoId: initialPage.repo_id,
        viewKey: initialPage.view_key,
      })
    : null
  const loadSelectedEntry = useCallback(
    (signal: AbortSignal) => loadHistoryEntry({
      data: {
        audience,
        entry: selectedEntryId ?? '',
        owner: params.owner,
        repo: params.repo,
      },
      signal,
    }),
    [audience, params.owner, params.repo, selectedEntryId],
  )
  const entryResource = useCachedResource({
    fallbackError: 'This history update is unavailable.',
    identity: entryIdentity,
    load: loadSelectedEntry,
    peek: peekHistoryEntryCache,
    read: readHistoryEntryCache,
    write: writeHistoryEntryCache,
  })
  const selectedEntry = entryResource.value
  const selectedFilePath = search.path ?? null
  const selectedFile = selectedEntry?.files.find(
    (file) => file.path === selectedFilePath,
  ) ?? null
  const diffIdentity = selectedEntryId && selectedFile
    ? historyEntryDiffCacheKey({
        audience,
        entry: selectedEntryId,
        generation: initialPage.generation,
        newOid: selectedFile.new_oid,
        oldOid: selectedFile.old_oid,
        path: selectedFile.path,
        repoId: initialPage.repo_id,
        viewKey: initialPage.view_key,
      })
    : null
  const loadSelectedDiff = useCallback(
    (signal: AbortSignal) => loadHistoryEntryFileDiff({
      data: {
        audience,
        entry: selectedEntryId ?? '',
        owner: params.owner,
        path: selectedFilePath ?? '',
        repo: params.repo,
      },
      signal,
    }),
    [audience, params.owner, params.repo, selectedEntryId, selectedFilePath],
  )
  const diffResource = useCachedResource({
    fallbackError: 'This file diff is unavailable.',
    identity: diffIdentity,
    load: loadSelectedDiff,
    peek: peekHistoryDiffCache,
    read: readHistoryDiffCache,
    write: writeHistoryDiffCache,
  })
  const detailState = historyEntryToCommitState(entryResource, initialPage)
  const fileDiffState: CommitFileDiffState =
    selectedFilePath && selectedEntry && !selectedFile
      ? { diff: null, error: 'This file is not part of the selected update.', status: 'failed' }
      : resourceToDiffState(diffResource)

  const replaceHistorySelection = useCallback((
    nextEntryId: string | null,
    nextPath: string | null = null,
  ) => {
    return navigate({
      params,
      replace: true,
      resetScroll: false,
      search: (current) => ({
        ...current,
        entry: nextEntryId ?? undefined,
        path: nextPath ?? undefined,
      }),
      to: '/$owner/$repo/history',
    })
  }, [navigate, params])

  const loadOlder = useCallback(async () => {
    const before = loaded.next_cursor
    if (!before || loadingOlder) return
    setLoadingOlder(true)
    setLoadOlderError(null)
    try {
      const page = await loadHistoryPage({
        data: { audience, before, owner: params.owner, repo: params.repo },
      })
      setLoaded((current) => appendHistoryPage(current, page, before))
    } catch (error) {
      setLoadOlderError(error instanceof Error ? error.message : 'Older history is unavailable.')
    } finally {
      setLoadingOlder(false)
    }
  }, [audience, loaded.next_cursor, loadingOlder, params.owner, params.repo])

  const closeDiff = useCallback(
    () => replaceHistorySelection(selectedEntryId),
    [replaceHistorySelection, selectedEntryId],
  )
  const selectAudience = useCallback(
    (nextAudience: ProjectionPreviewAudience) => navigate({
      params,
      replace: true,
      resetScroll: false,
      search: { audience: nextAudience },
      to: '/$owner/$repo/history',
    }),
    [navigate, params],
  )
  const selectEntry = useCallback(
    (entry: HistoryEntrySummary) =>
      replaceHistorySelection(entry.source_id),
    [replaceHistorySelection],
  )
  const selectFile = useCallback(
    (file: CommitFile) =>
      replaceHistorySelection(selectedEntryId, file.path),
    [replaceHistorySelection, selectedEntryId],
  )
  const saveDiffScroll = useCallback(
    (scrollTop: number) => writeHistoryDiffScroll(diffIdentity, scrollTop),
    [diffIdentity],
  )

  return {
    audience,
    availableAudiences,
    closeDiff,
    detailState,
    diffIdentity,
    entries: loaded.entries,
    fileDiffState,
    loadOlder,
    loadOlderError,
    loadingOlder,
    retryDetail: entryResource.retry,
    retryDiff: selectedFilePath && selectedEntry && !selectedFile
      ? undefined
      : diffResource.retry,
    saveDiffScroll,
    selectAudience,
    selectEntry,
    selectFile,
    selectedDetail: selectedEntry,
    selectedEntryId,
    selectedFilePath,
    showLoadOlder: loaded.next_cursor !== null,
  }
}

function historyEntryToCommitState(
  resource: CachedResource<HistoryEntryDetail>,
  page: HistoryPageResponse,
): CommitDetailState {
  if (resource.status === 'loaded') {
    return {
      commit: {
        audience: page.audience,
        author: resource.value.author,
        change_count: resource.value.file_change_count,
        files_truncated: false,
        files: resource.value.files,
        logical_commit_id: resource.value.source_id,
        message: resource.value.message,
        parent_projected_id: resource.value.parent_id,
        projected_id: resource.value.id,
        repo_id: page.repo_id,
        view_key: page.view_key,
      } satisfies CommitDetail,
      error: null,
      status: 'loaded',
    }
  }
  if (resource.status === 'failed') {
    return { commit: null, error: resource.error, status: 'failed' }
  }
  return { commit: null, error: null, status: resource.status }
}

function historyDetailCountLabel(detail: HistoryEntryDetail) {
  const visibilityCount = detail.visibility_summary.made_public_count
    + detail.visibility_summary.made_private_count
  const parts = []
  if (detail.kind !== 'visibility_change' && detail.file_change_count > 0) {
    parts.push(`${detail.file_change_count} file ${detail.file_change_count === 1 ? 'change' : 'changes'}`)
  }
  if (visibilityCount > 0) {
    parts.push(`${visibilityCount} visibility ${visibilityCount === 1 ? 'change' : 'changes'}`)
  }
  return parts.join(' · ')
}
