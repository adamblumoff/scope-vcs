import { RepoContentError } from '@/components/repo-content-error'
import { Outlet, createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/requests')({
  errorComponent: RepoContentError,
  component: () => <Outlet />,
})
