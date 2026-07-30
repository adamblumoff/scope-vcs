import { Outlet, createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/repos/$owner/$repo/runs')({
  component: () => <Outlet />,
})
