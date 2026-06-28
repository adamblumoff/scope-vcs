import type { RepoPublicationState } from '@/api/types'
import { Badge } from '@/components/ui/badge'

type BadgeVariant = 'success' | 'warning' | 'info'

const LIFECYCLE_VARIANT: Record<RepoPublicationState, BadgeVariant> = {
  PendingFirstPush: 'info',
  PendingPublish: 'warning',
  Published: 'success',
}

const LIFECYCLE_LABEL: Record<RepoPublicationState, string> = {
  PendingFirstPush: 'Setup',
  PendingPublish: 'Review needed',
  Published: 'Published',
}

export function LifecycleBadge({
  raw = false,
  state,
}: {
  raw?: boolean
  state: RepoPublicationState
}) {
  return (
    <Badge variant={LIFECYCLE_VARIANT[state]}>
      {raw ? state : LIFECYCLE_LABEL[state]}
    </Badge>
  )
}
