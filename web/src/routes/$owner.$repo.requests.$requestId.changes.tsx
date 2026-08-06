import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute(
  '/$owner/$repo/requests/$requestId/changes',
)({
  component: () => null,
})
