import { HttpError } from '@/api/client'
import { loadRepoFileForRequest } from '@/api/repos'
import type { RepoParams } from '@/api/types'
import { RepoContentError } from '@/components/repo-content-error'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import {
  DEFAULT_REPO_FILE_PATH,
  loadRepoFileWhenReady,
  type RepoFileLoadResult,
} from '@/features/repo-detail/repo-code-route-data'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
} from '@/lib/route-file'
import {
  createFileRoute,
  getRouteApi,
  useRouter,
} from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useEffect, useRef } from 'react'
import { toast } from 'sonner'

const PROJECTION_REBUILDING_MESSAGE = 'repository projection is rebuilding; retry shortly'
const codeRoute = getRouteApi('/$owner/$repo/_code')

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

export const Route = createFileRoute('/$owner/$repo/_code/')({
  validateSearch: parseRepoCodeSearch,
  loaderDeps: ({ search }) => ({
    file: search.file ?? DEFAULT_REPO_FILE_PATH,
  }),
  staleTime: Infinity,
  loader: async ({ abortController, deps, params }) => {
    const selectedFile = await loadRepoFileWhenReady({
      load: () => loadRepoFile({
        data: { ...params, path: deps.file },
        signal: abortController.signal,
      }),
      signal: abortController.signal,
    })

    return {
      selectedFile,
      selectedPath: selectedFile?.path ?? null,
    }
  },
  errorComponent: RepoContentError,
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const { selectedFile, selectedPath } = Route.useLoaderData()
  const { content } = codeRoute.useLoaderData()
  const params = Route.useParams()
  const router = useRouter()
  const latestSelection = useRef(0)
  const identity = selectedFile
    ? `${params.owner}\0${params.repo}\0${selectedFile.path}\0${selectedFile.oid}`
    : null

  useEffect(() => () => {
    latestSelection.current += 1
  }, [params.owner, params.repo])

  async function selectFile(path: string) {
    const selection = ++latestSelection.current
    const search = { file: displayRouteFilePath(path) }
    const matches = await router.preloadRoute({
      params,
      search,
      to: '/$owner/$repo',
    })
    const current = selection === latestSelection.current
    const ready = matches?.every(
      (match) => router.getMatch(match.id)?.status === 'success',
    )
    if (current && ready) {
      await router.navigate({
        params,
        resetScroll: false,
        search,
        to: '/$owner/$repo',
      })
      return true
    }
    if (current) toast.error('File could not be opened. Try again.')
    return false
  }

  return (
    <RepoDetailPage
      content={content}
      onSelectFilePath={selectFile}
      params={params}
      selectedFile={selectedFile}
      selectedFileIdentity={identity}
      selectedPath={selectedPath}
    />
  )
}

type RepoCodeSearch = { file?: string }
type RepoFileInput = RepoParams & { path: string }

function parseRepoCodeSearch(search: Record<string, unknown>): RepoCodeSearch {
  return { file: parseRouteFileSearch(search.file) }
}
