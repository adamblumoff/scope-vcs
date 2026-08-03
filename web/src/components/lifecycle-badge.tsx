import type { RepoLifecycleState } from '@/api/types'
import { Badge } from '@/components/ui/badge'

type BadgeVariant = 'success' | 'warning' | 'info'

const LIFECYCLE_VARIANT: Record<RepoLifecycleState, BadgeVariant> = {
  AwaitingFirstPush: 'info',
  Ready: 'success',
}

const LIFECYCLE_LABEL: Record<RepoLifecycleState, string> = {
  AwaitingFirstPush: 'Awaiting first push',
  Ready: 'Ready',
}

export function LifecycleBadge({
  raw = false,
  state,
}: {
  raw?: boolean
  state: RepoLifecycleState
}) {
  return (
    <Badge variant={LIFECYCLE_VARIANT[state]}>
      {raw ? state : LIFECYCLE_LABEL[state]}
    </Badge>
  )
}
