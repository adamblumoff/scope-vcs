import { Outlet, createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/runs')({
  component: () => <Outlet />,
})
