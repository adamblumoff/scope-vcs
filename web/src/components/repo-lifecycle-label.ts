import type { RepoLifecycleState } from '@/api/types'

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
