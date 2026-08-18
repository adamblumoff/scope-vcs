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
  displayRouteFilePath,
  parseRouteFileSearch,
} from '@/lib/route-file'
import { RepoContentError } from '@/components/repo-content-error'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

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
  loader: ({ abortController, deps, params }) =>
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
      requestedPath: deps.file,
    }),
  errorComponent: RepoContentError,
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const routeData = Route.useLoaderData()
  const { content, selectedFile, selectedPath } = routeData
  const params = Route.useParams()
  const { repo } = useRepoLayout()
  const navigate = useNavigate({ from: Route.fullPath })
  const identity = selectedFile
    ? [
        repo.id,
        repo.change_version,
        repo.access.can_read_private_files ? 'private' : 'public',
        selectedFile.path,
        selectedFile.oid,
      ].join('\0')
    : null

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
