import { createHash } from 'node:crypto'
import type { InvalidApiResponseError } from '../api/http'

const MAX_INVESTIGATION_GROUP_LENGTH = 200

export function invalidApiResponseGroup(error: InvalidApiResponseError) {
  const group = [
    error.requestMethod,
    error.requestPath,
    error.status,
    error.contentType ?? 'unknown',
  ].join(':')

  if (group.length <= MAX_INVESTIGATION_GROUP_LENGTH) return group

  return `sha256:${createHash('sha256').update(group).digest('hex')}`
}
