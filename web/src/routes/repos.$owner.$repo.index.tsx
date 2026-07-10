import {
  loadRepoFileForRequest,
  loadRepoForRequest,
  parseRepoParams,
} from '@/api/repos'
import type { RepoParams } from '@/api/types'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
  routeErrorMessage,
  selectedRouteFilePath,
} from '@/lib/route-file'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepo = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoForRequest(data))

const loadRepoFile = createServerFn({ method: 'GET' })
  .validator((data: RepoFileInput) => data)
  .handler(({ data }) => loadRepoFileForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/')({
  validateSearch: parseRepoCodeSearch,
  loaderDeps: ({ search }) => search,
  loader: async ({ deps: search, params }) => {
    const detail = await loadRepo({ data: params })
    const selectedPath = selectedRouteFilePath(detail.files, search.file)
    let selectedFile = null
    let selectedFileError = null
    if (selectedPath) {
      try {
        selectedFile = await loadRepoFile({
          data: { ...params, path: selectedPath },
        })
      } catch (error) {
        selectedFileError = routeErrorMessage(error, 'File content is unavailable.')
      }
    }
    return { detail, selectedFile, selectedFileError, selectedPath }
  },
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const { detail, selectedFile, selectedFileError, selectedPath } =
    Route.useLoaderData()
  const params = Route.useParams()
  const navigate = useNavigate({ from: Route.fullPath })

  return (
    <RepoDetailPage
      detail={detail}
      onSelectFile={(file) => {
        void navigate({ search: { file: displayRouteFilePath(file.path) } })
      }}
      params={params}
      selectedFile={selectedFile}
      selectedFileError={selectedFileError}
      selectedPath={selectedPath}
    />
  )
}

type RepoCodeSearch = { file?: string }
type RepoFileInput = RepoParams & { path: string }

function parseRepoCodeSearch(search: Record<string, unknown>): RepoCodeSearch {
  return { file: parseRouteFileSearch(search.file) }
}
