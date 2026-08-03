import { Outlet, createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner')({
  component: OwnerRoute,
})

function OwnerRoute() {
  return <Outlet />
}
