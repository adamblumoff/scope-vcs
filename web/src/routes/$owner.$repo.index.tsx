import { HttpError } from '@/api/client'
import {
  loadRepoContentForRequest,
  loadRepoFileForRequest,
  parseRepoParams,
} from '@/api/repos'
import type { RepoParams } from '@/api/types'
import { RepoDetailPage } from '@/features/repo-detail/repo-detail-page'
import { createRepoCodeRouteHandoff } from '@/features/repo-detail/repo-code-route-handoff'
import { loadRepoCodeRouteData } from '@/features/repo-detail/repo-code-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import {
  displayRouteFilePath,
  parseRouteFileSearch,
} from '@/lib/route-file'
import { RepoContentError } from '@/components/repo-content-error'
import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useEffect } from 'react'

const repoCodeRouteHandoff = typeof window === 'undefined'
  ? null
  : createRepoCodeRouteHandoff()

const loadRepoContent = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoContentForRequest(data))

const loadRepoFile = createServerFn({ method: 'GET' })
  .validator((data: RepoFileInput) => data)
  .handler(async ({ data }) => {
    try {
      return await loadRepoFileForRequest(data)
    } catch (error) {
      if (error instanceof HttpError && error.status === 404) return null
      throw error
    }
  })

export const Route = createFileRoute('/$owner/$repo/')({
  validateSearch: parseRepoCodeSearch,
  loaderDeps: ({ search }) => ({ file: search.file }),
  staleTime: Infinity,
  loader: ({ deps, params }) =>
    repoCodeRouteHandoff?.take({ ...params, path: deps.file })
    ?? loadRepoCodeRouteData({
      loadContent: () => loadRepoContent({ data: params }),
      loadFile: (path) => loadRepoFile({
        data: { ...params, path },
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
  const search = Route.useSearch()
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

  useEffect(() => {
    repoCodeRouteHandoff?.stage(
      { ...params, path: search.file },
      routeData,
    )
  }, [params, routeData, search.file])

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
