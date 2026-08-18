import { loadRepoContentForRequest, parseRepoParams } from '@/api/repos'
import { createFileRoute, Outlet } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRepoContent = createServerFn({ method: 'GET' })
  .validator(parseRepoParams)
  .handler(({ data }) => loadRepoContentForRequest(data))

export const Route = createFileRoute('/$owner/$repo/_code')({
  loader: ({ params }) => ({
    content: loadRepoContent({ data: params }),
  }),
  component: RepoCodeLayoutRoute,
})

function RepoCodeLayoutRoute() {
  return <Outlet />
}
