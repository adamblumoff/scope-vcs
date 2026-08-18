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
import {
  displayRouteFilePath,
  parseRouteFileSearch,
} from '@/lib/route-file'
import { RepoContentError } from '@/components/repo-content-error'
import { createFileRoute, useRouter } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useEffect, useRef } from 'react'

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
  loaderDeps: ({ search }) => ({
    file: search.file ?? DEFAULT_REPO_FILE_PATH,
  }),
  staleTime: Infinity,
  loader: async ({ abortController, cause, deps, params }) => {
    const data = await loadRepoCodeRouteData({
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
      requestedPath: deps.file,
    })

    // Initial entry can reveal the primary file before the tree. Preloads wait
    // for both so the cached navigation can replace the screen in one pass.
    if (cause !== 'enter') await data.content
    return data
  },
  errorComponent: RepoContentError,
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const { content, selectedFile, selectedPath } = Route.useLoaderData()
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
    let committed = false
    await router.preloadRoute({ params, search, to: '/$owner/$repo' })
    if (selection === latestSelection.current) {
      await router.navigate({
        params,
        resetScroll: false,
        search,
        to: '/$owner/$repo',
      })
      committed = true
    }
    return committed
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
