import type { RepoDetail } from '@/api/types'

export function shouldPollPendingFirstPush(
  detail: RepoDetail | null,
  childMatchCount: number,
) {
  return (
    childMatchCount === 0 &&
    detail?.repo.lifecycle_state === 'PendingFirstPush'
  )
}
