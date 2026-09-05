import { createFileRoute, Outlet } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/_code')({
  loader: async ({ parentMatchPromise }) => (await parentMatchPromise).loaderData,
  component: RepoCodeLayoutRoute,
})

function RepoCodeLayoutRoute() {
  return <Outlet />
}
