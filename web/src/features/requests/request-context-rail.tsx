import type { RequestSummary } from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { Coins, GitBranch, GitCommitHorizontal } from 'lucide-react'
import type { ReactNode } from 'react'
import {
  formatUnixDate,
  requestAudienceLabel,
  requestAuthorRoleLabel,
  shortOid,
} from './request-labels'

export function RequestContextRail({ request }: { request: RequestSummary }) {
  return (
    <aside className="min-w-0 border-l border-border bg-muted/15">
      <RailSection icon={<GitBranch />} title="Request">
        <RailValue label="Branch" value={`origin/${request.name}`} />
        <div className="flex flex-wrap gap-1.5">
          <Badge variant="outline">{requestAudienceLabel(request)}</Badge>
          <Badge variant="outline">{requestAuthorRoleLabel(request)}</Badge>
        </div>
      </RailSection>

      <RailSection icon={<GitCommitHorizontal />} title="Git state">
        <RailValue label="Base" value={shortOid(request.base_main_oid)} />
        <RailValue label="Head" value={shortOid(request.head_oid)} />
        <pre className="overflow-x-auto rounded-md bg-muted px-3 py-2 text-[11px] leading-5"><code>{`git fetch origin\ngit switch --track origin/${request.name}`}</code></pre>
      </RailSection>

      <RailSection icon={<Coins />} title="Review">
        <RailValue
          label="Current stake"
          value={`${request.current_stake_credits} credits`}
        />
        <RailValue
          label="First published"
          value={formatUnixDate(request.first_ready_at_unix)}
        />
        <RailValue
          label="Ready since"
          value={formatUnixDate(request.ready_at_unix)}
        />
        <RailValue
          label="Held since"
          value={formatUnixDate(request.held_at_unix)}
        />
        <RailValue
          label="Assessment"
          value={request.assessment_outcome ?? 'Not assessed'}
        />
        <RailValue
          label="Completed"
          value={formatUnixDate(request.completed_at_unix)}
        />
        <RailValue
          label="Merged"
          value={formatUnixDate(request.merged_at_unix)}
        />
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
      <span className="text-[11px] font-medium text-muted-foreground">
        {label}
      </span>
      <span className="break-all font-mono text-xs">{value}</span>
    </div>
  )
}
