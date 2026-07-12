import {
  loadRepoFileForRequest,
  loadRepoContentForRequest,
  parseRepoParams,
} from '@/api/repos'
import type { RepoParams } from '@/api/types'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import {
  defaultReadmePath,
  displayRouteFilePath,
  parseRouteFileSearch,
  routeErrorMessage,
  selectedRouteFilePath,
} from '@/lib/route-file'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepoContent = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoContentForRequest(data))

const loadRepoFile = createServerFn({ method: 'GET' })
  .validator((data: RepoFileInput) => data)
  .handler(({ data }) => loadRepoFileForRequest(data))

export const Route = createFileRoute('/repos/$owner/$repo/')({
  validateSearch: parseRepoCodeSearch,
  loaderDeps: ({ search }) => search,
  loader: async ({ deps: search, params }) => {
    const content = await loadRepoContent({ data: params })
    const selectedPath = search.empty
      ? null
      : selectedRouteFilePath(
          content.files,
          search.file ?? defaultReadmePath(content.files),
        )
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
    return { content, selectedFile, selectedFileError, selectedPath }
  },
  component: RepoIndexRoute,
})

function RepoIndexRoute() {
  const { content, selectedFile, selectedFileError, selectedPath } =
    Route.useLoaderData()
  const params = Route.useParams()
  const navigate = useNavigate({ from: Route.fullPath })

  return (
    <RepoDetailPage
      content={content}
      onSelectFilePath={(path) => {
        void navigate({
          search: path
            ? { empty: undefined, file: displayRouteFilePath(path) }
            : { empty: true, file: undefined },
        })
      }}
      params={params}
      selectedFile={selectedFile}
      selectedFileError={selectedFileError}
      selectedPath={selectedPath}
    />
  )
}

type RepoCodeSearch = { empty?: true; file?: string }
type RepoFileInput = RepoParams & { path: string }

function parseRepoCodeSearch(search: Record<string, unknown>): RepoCodeSearch {
  return {
    empty: search.empty === true || search.empty === 'true' ? true : undefined,
    file: parseRouteFileSearch(search.file),
  }
}
