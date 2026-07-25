import type { RequestSummary } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Coins, GitBranch, GitCommitHorizontal } from 'lucide-react'
import type { ReactNode } from 'react'
import { RequestInvitees } from './request-invitees'
import {
  formatUnixDate,
  requestAudienceLabel,
  requestAuthorRoleLabel,
  shortOid,
} from './request-labels'
import type { RequestActionController } from './use-request-actions'

export function RequestContextRail({
  actions,
  request,
}: {
  actions: RequestActionController
  request: RequestSummary
}) {
  const accepted = request.assessment_previews.find((item) => item.outcome === 'Accepted')

  return (
    <aside className="order-1 min-w-0 border-b border-border bg-muted/15 xl:order-2 xl:border-b-0 xl:border-l">
      <RailSection icon={<GitBranch />} title="Request details">
        <RailValue label="Branch" value={`origin/${request.name}`} />
        <div className="flex flex-wrap gap-1.5">
          <Badge variant="outline">{requestAudienceLabel(request)}</Badge>
          <Badge variant="outline">{requestAuthorRoleLabel(request)}</Badge>
        </div>
      </RailSection>

      <RequestInvitees actions={actions} request={request} />

      <RailSection icon={<Coins />} title="Review">
        <RailValue
          label="Current stake"
          value={`${request.current_stake_credits} credits`}
        />
        {accepted && request.state === 'ReadyForReview' ? (
          <p className="text-xs leading-5 text-muted-foreground">
            Accepted returns {accepted.refunded_credits} and rewards {accepted.reward_credits} credits.
          </p>
        ) : null}
        <RailValue
          label="First published"
          value={formatUnixDate(request.first_ready_at_unix)}
        />
        <RailValue label="Ready since" value={formatUnixDate(request.ready_at_unix)} />
        <RailValue label="Held since" value={formatUnixDate(request.held_at_unix)} />
        <RailValue
          label="Assessment"
          value={request.assessment_outcome ?? 'Not assessed'}
        />
        {request.assessment_body_markdown ? (
          <p className="whitespace-pre-wrap text-xs leading-5 text-muted-foreground">
            {request.assessment_body_markdown}
          </p>
        ) : null}
        <RailValue label="Completed" value={formatUnixDate(request.completed_at_unix)} />
        <RailValue label="Merged" value={formatUnixDate(request.merged_at_unix)} />
      </RailSection>

      <RailSection icon={<GitCommitHorizontal />} title="Git state">
        <RailValue label="Base" value={shortOid(request.base_main_oid)} />
        <RailValue label="Head" value={shortOid(request.head_oid)} />
        <pre className="overflow-x-auto rounded-md bg-muted px-3 py-2 text-[11px] leading-5"><code>{`git fetch origin\ngit switch --track origin/${request.name}`}</code></pre>
      </RailSection>
    </aside>
  )
}

function RailSection({
  children,
  icon,
  title,
}: {
  children: ReactNode
  icon: ReactNode
  title: string
}) {
  return (
    <section className="border-b border-border px-5 py-5">
      <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.08em] text-muted-foreground [&_svg]:size-3.5">
        {icon}
        <h2>{title}</h2>
      </div>
      <div className="mt-3 grid gap-3">{children}</div>
    </section>
  )
}

function RailValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid gap-1">
      <span className="text-[11px] font-medium text-muted-foreground">{label}</span>
      <span className="break-all font-mono text-xs">{value}</span>
    </div>
  )
}
