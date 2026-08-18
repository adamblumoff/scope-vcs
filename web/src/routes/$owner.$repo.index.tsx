import { HttpError } from '@/api/client'
import {
  loadRepoContentForRequest,
  loadRepoFileForRequest,
  parseRepoParams,
} from '@/api/repos'
import type { RepoParams } from '@/api/types'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import {
  DEFAULT_REPO_FILE_PATH,
  loadRepoCodeRouteData,
  loadRepoFileWhenReady,
  type RepoFileLoadResult,
} from '@/features/repo-detail/repo-code-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import {
  peekRepoFileCache,
  readRepoFileCache,
  repoFileCacheKey,
  writeRepoFileCache,
} from '@/features/repo-detail/repo-file-cache'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
} from '@/lib/route-file'
import { useCachedResource } from '@/lib/use-cached-resource'
import { RepoContentError } from '@/components/repo-content-error'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const PROJECTION_REBUILDING_MESSAGE = 'repository projection is rebuilding; retry shortly'

const loadRepoContent = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoContentForRequest(data))

const loadRepoFile = createServerFn({ method: 'GET' })
  .validator((data: RepoFileInput) => data)
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

export const Route = createFileRoute('/$owner/$repo/')({
  validateSearch: parseRepoCodeSearch,
  beforeLoad: ({ search }) => ({
    initialFilePath: search.file ?? DEFAULT_REPO_FILE_PATH,
  }),
  staleTime: Infinity,
  loader: ({ abortController, context, params }) =>
    loadRepoCodeRouteData({
      loadContent: () => loadRepoContent({
        data: params,
        signal: abortController.signal,
      }),
      loadFile: (path) => loadRepoFileWhenReady({
        load: () => loadRepoFile({
          data: { ...params, path },
          signal: abortController.signal,
        }),
        signal: abortController.signal,
      }),
      requestedPath: context.initialFilePath,
    }),
  errorComponent: RepoContentError,
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const routeData = Route.useLoaderData()
  const {
    content,
    requestedPath: initialRequestedPath,
    selectedFile: initialFile,
    selectedPath: initialPath,
  } = routeData
  const params = Route.useParams()
  const search = Route.useSearch()
  const { repo } = useRepoLayout()
  const navigate = useNavigate({ from: Route.fullPath })
  const requestedPath = search.file ?? DEFAULT_REPO_FILE_PATH
  const identity = repoFileCacheKey({
    audience: repo.access.can_read_private_files ? 'private' : 'public',
    changeVersion: repo.change_version,
    path: requestedPath,
    repoId: repo.id,
  })
  const initialIdentity = repoFileCacheKey({
    audience: repo.access.can_read_private_files ? 'private' : 'public',
    changeVersion: repo.change_version,
    path: initialRequestedPath,
    repoId: repo.id,
  })
  const readFile = useCallback(
    (key: string) => readRepoFileCache(key)
      ?? (key === initialIdentity ? initialFile : null),
    [initialFile, initialIdentity],
  )
  const peekFile = useCallback(
    (key: string) => peekRepoFileCache(key)
      ?? (key === initialIdentity ? initialFile : null),
    [initialFile, initialIdentity],
  )
  const loadSelectedFile = useCallback(
    async (signal: AbortSignal) => {
      const file = await loadRepoFileWhenReady({
        load: () => loadRepoFile({
          data: {
            owner: params.owner,
            path: requestedPath,
            repo: params.repo,
          },
          signal,
        }),
        signal,
      })
      if (!file) {
        throw new Error('This file is no longer available in the current scoped view.')
      }
      return file
    },
    [params.owner, params.repo, requestedPath],
  )
  const resourceIdentity = identity === initialIdentity && !initialFile
    ? null
    : identity
  const selectedResource = useCachedResource({
    fallbackError: 'File content is unavailable.',
    identity: resourceIdentity,
    load: loadSelectedFile,
    peek: peekFile,
    read: readFile,
    write: writeRepoFileCache,
  })
  const selectedPath = identity === initialIdentity
    ? initialPath
    : selectedResource.value?.path ?? `/${displayRouteFilePath(requestedPath)}`

  return (
    <RepoDetailPage
      content={content}
      onSelectFilePath={(path) => {
        void navigate({
          resetScroll: false,
          search: { file: displayRouteFilePath(path) },
        })
      }}
      params={params}
      selectedFile={selectedResource.value}
      selectedFileError={selectedResource.error}
      selectedFileIdentity={resourceIdentity}
      selectedFileLoading={selectedResource.status === 'loading'}
      selectedFileRetry={selectedResource.retry}
      selectedPath={selectedPath}
    />
  )
}

type RepoCodeSearch = { file?: string }
type RepoFileInput = RepoParams & { path: string }

function parseRepoCodeSearch(search: Record<string, unknown>): RepoCodeSearch {
  return { file: parseRouteFileSearch(search.file) }
}
