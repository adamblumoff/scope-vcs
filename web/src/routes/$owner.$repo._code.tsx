import { createFileRoute, Outlet } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/_code')({
  component: RepoCodeLayoutRoute,
})

function RepoCodeLayoutRoute() {
  return <Outlet />
}
