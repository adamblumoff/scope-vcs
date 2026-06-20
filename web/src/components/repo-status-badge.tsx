import type { RepoLifecycleState } from '@/api/types'
import { Badge } from '@/components/ui/badge'

export function RepoStatusBadge({ state }: { state: RepoLifecycleState }) {
  return <Badge variant="outline">{lifecycleLabel(state)}</Badge>
}

export function lifecycleLabel(state: RepoLifecycleState) {
  switch (state) {
    case 'PendingFirstPush':
      return 'Pending first push'
    case 'PendingPublish':
      return 'Pending publish'
    case 'Published':
      return 'Published'
  }
}
