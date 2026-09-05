import { parseRepoFileInput } from '@/api/request-inputs'
import { HttpError } from '@/api/client'
import {
  loadRepoContentForRequest,
  loadRepoFileForRequest,
  parseRepoParams,
} from '@/api/repos'
import type { RepoContent, RepoFileContent, RepoParams } from '@/api/types'
import { RepoContentError } from '@/components/repo-content-error'
import {
  peekRepoContentCache,
  readRepoContentCache,
  repoContentCacheKey,
  writeRepoContentCache,
} from '@/features/repo-detail/repo-content-cache'
import {
  peekRepoFileCache,
  readRepoFileCache,
  repoFileCacheKey,
  writeRepoFileCache,
} from '@/features/repo-detail/repo-file-cache'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import { RepositoryCodePending } from '@/features/repo-detail/repository-code-pending'
import {
  DEFAULT_REPO_FILE_PATH,
  loadRepoFileWhenReady,
  type RepoFileLoadResult,
} from '@/features/repo-detail/repo-code-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
  selectedRouteFilePath,
} from '@/lib/route-file'
import { useCachedResource } from '@/lib/use-cached-resource'
import {
  createFileRoute,
  useNavigate,
} from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const PROJECTION_REBUILDING_MESSAGE = 'repository projection is rebuilding; retry shortly'

const loadRepoContent = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoContentForRequest(data))

const loadRepoFile = createServerFn({ method: 'GET' })
  .validator(parseRepoFileInput)
  .handler(async ({ data }): Promise<RepoFileLoadResult> => {
    try {
      return { file: await loadRepoFileForRequest(data), status: 'ready' }
    } catch (error) {
      if (error instanceof HttpError && error.status === 404) {
        return { status: 'missing' }
      }
      if (
        error instanceof HttpError &&
        error.status === 503 &&
        error.message === PROJECTION_REBUILDING_MESSAGE
      ) {
        return { status: 'rebuilding' }
      }
      throw error
    }
  })

export const Route = createFileRoute('/$owner/$repo/_code/')({
  validateSearch: parseRepoCodeSearch,
  errorComponent: RepoContentError,
  pendingComponent: RepositoryCodePending,
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const params = Route.useParams()
  const { repo } = useRepoLayout()
  const search = Route.useSearch()
  const navigate = useNavigate({ from: Route.fullPath })
  const selectedPath = search.file ?? DEFAULT_REPO_FILE_PATH
  const owner = params.owner
  const repoName = params.repo
  const contentIdentity = repoContentCacheKey({
    audience: repo.access.can_read_private_files ? 'private' : 'public',
    changeVersion: repo.change_version,
    repoId: repo.id,
  })
  const loadContent = useCallback(
    (signal: AbortSignal): Promise<RepoContent> => loadRepoContent({
      data: { owner, repo: repoName },
      signal,
    }),
    [owner, repoName],
  )
  const contentResource = useCachedResource({
    fallbackError: 'Repository files are unavailable.',
    identity: contentIdentity,
    load: loadContent,
    peek: peekRepoContentCache,
    read: readRepoContentCache,
    write: writeRepoContentCache,
  })
  const content = contentResource.value
  const selectedFilePath = selectedRouteFilePath(
    content?.files ?? [],
    selectedPath,
  )
  const selectedMeta = content?.files.find(
    (file) => file.path === selectedFilePath,
  )
  const selectedFileIdentity = selectedMeta && selectedFilePath
    ? repoFileCacheKey({
        audience: repo.access.can_read_private_files ? 'private' : 'public',
        changeVersion: repo.change_version,
        oid: selectedMeta.oid,
        path: selectedFilePath,
        repoId: repo.id,
      })
    : null
  const loadFile = useCallback(
    async (path: string, signal: AbortSignal): Promise<RepoFileContent> => {
      const file = await loadRepoFileWhenReady({
        load: () => loadRepoFile({
          data: { owner, path, repo: repoName },
          signal,
        }),
        signal,
      })
      if (!file) {
        throw new Error(
          'This file is no longer available in the current scoped view.',
        )
      }
      return file
    },
    [owner, repoName],
  )
  const loadSelectedFile = useCallback(
    (signal: AbortSignal) => loadFile(selectedFilePath ?? '', signal),
    [loadFile, selectedFilePath],
  )
  const selectedFileResource = useCachedResource({
    fallbackError: 'File content is unavailable.',
    identity: selectedFileIdentity,
    load: loadSelectedFile,
    peek: peekRepoFileCache,
    read: readRepoFileCache,
    write: writeRepoFileCache,
  })
  const selectFile = useCallback((path: string) => {
    const nextPath = displayRouteFilePath(path)
    if (nextPath === displayRouteFilePath(selectedPath)) return
    void navigate({
      resetScroll: false,
      search: {
        file: nextPath === DEFAULT_REPO_FILE_PATH ? undefined : nextPath,
      },
    })
  }, [navigate, selectedPath])

  return (
    <RepoDetailPage
      content={content}
      contentError={contentResource.error}
      contentLoading={contentResource.status === 'loading'}
      contentRetry={contentResource.retry}
      onSelectFilePath={selectFile}
      params={params}
      repo={repo}
      selectedFile={selectedFileResource.value}
      selectedFileError={selectedFileResource.error}
      selectedFileIdentity={selectedFileIdentity}
      selectedFileLoading={selectedFileResource.status === 'loading'}
      selectedFileRetry={selectedFileResource.retry}
      selectedPath={content
        ? selectedFilePath ? displayRouteFilePath(selectedFilePath) : null
        : selectedPath}
    />
  )
}

type RepoCodeSearch = { file?: string }

function parseRepoCodeSearch(search: Record<string, unknown>): RepoCodeSearch {
  return { file: parseRouteFileSearch(search.file) }
}
