import type { RequestParams, RequestRevisions } from '@/api/types'

type RequestRevisionLoadIdentity = RequestParams & {
  commit_oid?: string
  revision_id?: string
}

export function createRequestRevisionRedirectHandoff() {
  let pending: {
    key: string
    revisions: RequestRevisions
  } | null = null

  return {
    stage(input: RequestRevisionLoadIdentity, revisions: RequestRevisions) {
      pending = { key: requestRevisionLoadKey(input), revisions }
    },
    take(input: RequestRevisionLoadIdentity) {
      const staged = pending
      pending = null
      return staged?.key === requestRevisionLoadKey(input)
        ? staged.revisions
        : null
    },
  }
}

function requestRevisionLoadKey(input: RequestRevisionLoadIdentity) {
  return [
    input.owner,
    input.repo,
    input.request_id,
    input.revision_id ?? '',
    input.commit_oid ?? '',
  ].join('\0')
}
