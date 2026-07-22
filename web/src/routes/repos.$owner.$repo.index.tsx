import { HttpError } from '@/api/client'
import {
  loadRepoContentForRequest,
  loadRepoFileForRequest,
  parseRepoParams,
} from '@/api/repos'
import type { RepoFileContent, RepoParams } from '@/api/types'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import {
  peekRepoFileCache,
  readRepoFileCache,
  repoFileCacheKey,
  writeRepoFileCache,
} from '@/features/repo-detail/repo-file-cache'
import { useCachedResource } from '@/lib/use-cached-resource'
import {
  defaultReadmePath,
  displayRouteFilePath,
  parseRouteFileSearch,
  selectedRouteFilePath,
} from '@/lib/route-file'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback } from 'react'

const PROJECTION_REBUILDING_MESSAGE = 'repository projection is rebuilding; retry shortly'

const loadRepoContent = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoContentForRequest(data))

const loadRepoFile = createServerFn({ method: 'GET' })
  .validator((data: RepoFileInput) => data)
  .handler(async ({ data }) => {
    try {
      return { file: await loadRepoFileForRequest(data), status: 'ready' } as const
    } catch (error) {
      if (
        error instanceof HttpError &&
        error.status === 503 &&
        error.message === PROJECTION_REBUILDING_MESSAGE
      ) {
        return { file: null, status: 'rebuilding' } as const
      }
      throw error
    }
  })

export const Route = createFileRoute('/repos/$owner/$repo/')({
  validateSearch: parseRepoCodeSearch,
  staleTime: Infinity,
  loader: ({ params }) => loadRepoContent({ data: params }),
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const content = Route.useLoaderData()
  const params = Route.useParams()
  const search = Route.useSearch()
  const { repo } = useRepoLayout()
  const navigate = useNavigate({ from: Route.fullPath })
  const selectedPath = search.empty
    ? null
    : selectedRouteFilePath(
        content.files,
        search.file ?? defaultReadmePath(content.files),
      )
  const selectedMeta = content.files.find((file) => file.path === selectedPath)
  const identity = selectedMeta && selectedPath
    ? repoFileCacheKey({
        audience: repo.access.can_read_private_files ? 'private' : 'public',
        changeVersion: repo.change_version,
        oid: selectedMeta.oid,
        path: selectedPath,
        repoId: repo.id,
      })
    : null
  const loadSelectedFile = useCallback(
    (signal: AbortSignal) => loadRepoFileWhenReady({
      data: {
        owner: params.owner,
        path: selectedPath ?? '',
        repo: params.repo,
      },
      signal,
    }),
    [params.owner, params.repo, selectedPath],
  )
  const selectedResource = useCachedResource({
    fallbackError: 'File content is unavailable.',
    identity,
    load: loadSelectedFile,
    peek: peekRepoFileCache,
    read: readRepoFileCache,
    write: writeRepoFileCache,
  })

  return (
    <RepoDetailPage
      content={content}
      onSelectFilePath={(path) => {
        void navigate({
          resetScroll: false,
          search: path
            ? { empty: undefined, file: displayRouteFilePath(path) }
            : { empty: true, file: undefined },
        })
      }}
      params={params}
      selectedFile={selectedResource.value}
      selectedFileError={selectedResource.error}
      selectedFileIdentity={identity}
      selectedFileLoading={selectedResource.status === 'loading'}
      selectedFileRetry={selectedResource.retry}
      selectedPath={selectedPath}
    />
  )
}

async function loadRepoFileWhenReady({
  data,
  signal,
}: {
  data: RepoFileInput
  signal: AbortSignal
}): Promise<RepoFileContent> {
  const retryDelays = [0, 250, 500, 1_000, 2_000]

  async function attempt(index: number): Promise<RepoFileContent> {
    const delay = retryDelays[index]
    if (delay === undefined) {
      throw new Error('Repository projection is still rebuilding. Try again shortly.')
    }
    if (delay > 0) await abortableDelay(delay, signal)
    const result = await loadRepoFile({ data, signal })
    return result.status === 'ready' ? result.file : attempt(index + 1)
  }

  return attempt(0)
}

function abortableDelay(delay: number, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    if (signal.aborted) {
      reject(new DOMException('The request was aborted.', 'AbortError'))
      return
    }
    const onAbort = () => {
      window.clearTimeout(timeout)
      reject(new DOMException('The request was aborted.', 'AbortError'))
    }
    const timeout = window.setTimeout(() => {
      signal.removeEventListener('abort', onAbort)
      resolve()
    }, delay)
    signal.addEventListener('abort', onAbort, { once: true })
  })
}

type RepoCodeSearch = { empty?: true; file?: string }
type RepoFileInput = RepoParams & { path: string }

function parseRepoCodeSearch(search: Record<string, unknown>): RepoCodeSearch {
  return {
    empty: search.empty === true || search.empty === 'true' ? true : undefined,
    file: parseRouteFileSearch(search.file),
  }
}
