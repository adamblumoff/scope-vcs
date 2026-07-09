import {
  loadRepoFileForRequest,
  loadRepoForRequest,
  parseRepoParams,
} from '@/api/repos'
import type { RepoDetail, RepoParams } from '@/api/types'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
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
    const selectedPath = selectedRepoPath(detail.files, search.file)
    let selectedFile = null
    let selectedFileError = null
    if (selectedPath) {
      try {
        selectedFile = await loadRepoFile({
          data: { ...params, path: selectedPath },
        })
      } catch (error) {
        selectedFileError = errorMessage(error, 'File content is unavailable.')
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
        void navigate({ search: { file: displayRepoPath(file.path) } })
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
  return { file: searchFilePath(search.file) }
}

function searchFilePath(value: unknown) {
  if (typeof value !== 'string') return undefined
  const path = displayRepoPath(value)
  return path && !path.split('/').some((segment) => segment === '..' || segment === '.')
    ? path
    : undefined
}

function selectedRepoPath(files: RepoDetail['files'], selected?: string) {
  if (!selected) return null
  return files.find((file) => displayRepoPath(file.path) === selected)?.path ?? null
}

function displayRepoPath(path: string) {
  return path.replace(/^\/+/, '')
}

function errorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim() ? error.message : fallback
}
