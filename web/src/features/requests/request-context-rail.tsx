import type { RequestParams, RequestRating, RequestRatings, RequestSummary } from '@/api/types'
import { GitCommitHorizontal } from 'lucide-react'
import type { ReactNode } from 'react'
import { RequestInvitees } from './request-invitees'
import { RequestRatingsSection } from './request-ratings-section'
import type { RateRequestInput } from '@/api/requests'
import {
  formatUnixDate,
  requestAuthorRoleLabel,
  shortOid,
} from './request-labels'
import type { RequestActionController } from './use-request-actions'

export function RequestContextRail({
  actions,
  onRate,
  params,
  ratings,
  request,
}: {
  actions: RequestActionController
  onRate: (input: RateRequestInput) => Promise<RequestRating>
  params: RequestParams
  ratings: RequestRatings
  request: RequestSummary
}) {
  return (
    <aside className="order-1 min-w-0 border-b border-border px-5 py-6 xl:order-2 xl:border-b-0 xl:border-l">
      <div className="grid min-w-0 gap-7">
        <RailSection title="Lifecycle">
          <RailValue label="Author" value={requestAuthorRoleLabel(request)} />
          <RailValue label="Submitted" value={formatUnixDate(request.submitted_at_unix)} />
          {request.closed_at_unix !== null && (
            <RailValue label="Closed" value={formatUnixDate(request.closed_at_unix)} />
          )}
          {request.merged_at_unix !== null && (
            <RailValue label="Merged" value={formatUnixDate(request.merged_at_unix)} />
          )}
        </RailSection>

        <RequestInvitees actions={actions} request={request} />

        <RequestRatingsSection initial={ratings} onRate={onRate} params={params} />

        <RailSection icon={<GitCommitHorizontal />} title="Git state">
          <RailValue label="Base" value={shortOid(request.base_main_oid)} />
          <RailValue label="Head" value={shortOid(request.head_oid)} />
          <pre className="mt-1 min-w-0 whitespace-pre-wrap break-all rounded-md bg-muted px-3 py-2 text-[11px] leading-5"><code>{`git fetch origin\ngit switch --track origin/${request.name}`}</code></pre>
        </RailSection>
      </div>
    </aside>
  )
}

function RailSection({
  children,
  icon,
  title,
}: {
  children: ReactNode
  icon?: ReactNode
  title: string
}) {
  return (
    <section>
      <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.08em] text-muted-foreground [&_svg]:size-3.5">
        {icon}
        <h2>{title}</h2>
      </div>
      <div className="mt-3 grid min-w-0 gap-2.5">{children}</div>
    </section>
  )
}

function RailValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-xs">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 break-all text-right font-mono">{value}</span>
    </div>
  )
}
