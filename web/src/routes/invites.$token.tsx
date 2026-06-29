import {
  acceptRepoInviteForRequest,
  loadRepoInviteForRequest,
  parseRepoInviteTokenInput,
} from '@/api/repos'
import { InvitePage } from '@/features/invites/invite-page'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadInvite = createServerFn({ method: 'GET' })
  .validator(parseRepoInviteTokenInput)
  .handler(({ data }) => loadRepoInviteForRequest(data))

const acceptInvite = createServerFn({ method: 'POST' })
  .validator(parseRepoInviteTokenInput)
  .handler(({ data }) => acceptRepoInviteForRequest(data))

export const Route = createFileRoute('/invites/$token')({
  loader: ({ params }) => loadInvite({ data: params }),
  component: InviteRoute,
})

function InviteRoute() {
  const invite = Route.useLoaderData()
  const params = Route.useParams()
  return (
    <InvitePage
      acceptInvite={(input) => acceptInvite({ data: input })}
      invite={invite}
      token={params.token}
    />
  )
}
