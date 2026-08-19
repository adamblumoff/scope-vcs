import {
  acceptRepoInviteForRequest,
  loadRepoInviteForRequest,
  parseRepoInviteTokenInput,
} from '@/api/repos'
import { ApplicationPendingShell } from '@/components/pending-surface'
import { Skeleton } from '@/components/ui/skeleton'
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
  pendingComponent: InvitePending,
  component: InviteRoute,
})

function InvitePending() {
  return (
    <ApplicationPendingShell
      contextLabel="Repository invite"
      label="Loading repository invite"
    >
      <div className="py-8 lg:py-10">
        <h1 className="text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[32px]">
          Repository invite
        </h1>
        <Skeleton className="mt-3 h-5 w-48" />
        <Skeleton className="mt-2 h-3 w-64 max-w-full" />
        <div className="mt-6 divide-y divide-border">
          {['Access', 'Continue'].map((title, index) => (
            <section
              className="grid gap-4 py-5 md:grid-cols-[220px_minmax(0,1fr)]"
              key={title}
            >
              <div>
                <div className="text-sm font-semibold leading-5">{title}</div>
                <Skeleton className="mt-2 h-3 w-44 max-w-full" />
                <Skeleton className="mt-1.5 h-3 w-36 max-w-4/5" />
              </div>
              <div className="space-y-3">
                <Skeleton className="h-8 w-full" />
                <Skeleton className="h-8" style={{ width: index ? '46%' : '100%' }} />
              </div>
            </section>
          ))}
        </div>
      </div>
    </ApplicationPendingShell>
  )
}

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
