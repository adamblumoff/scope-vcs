import type { CommitDetail, ReviewFileDiff } from '@/api/types'
import type { CachedResource } from '@/lib/use-cached-resource'

export type CommitDetailState =
  | { commit: null; error: null; status: 'idle' }
  | { commit: null; error: null; status: 'loading' }
  | { commit: CommitDetail; error: null; status: 'loaded' }
  | { commit: null; error: string; status: 'failed' }

export type CommitFileDiffState =
  | { diff: null; error: null; status: 'idle' }
  | { diff: null; error: null; status: 'loading' }
  | { diff: ReviewFileDiff; error: null; status: 'loaded' }
  | { diff: null; error: string; status: 'failed' }

export function resourceToDiffState(
  resource: CachedResource<ReviewFileDiff>,
): CommitFileDiffState {
  switch (resource.status) {
    case 'idle':
      return { diff: null, error: null, status: 'idle' }
    case 'loading':
      return { diff: null, error: null, status: 'loading' }
    case 'loaded':
      return { diff: resource.value, error: null, status: 'loaded' }
    case 'failed':
      return { diff: null, error: resource.error, status: 'failed' }
  }
}
