import {
  loadCommitDetailForRequest,
  loadCommitFileDiffForRequest,
  loadCommitHistoryForRequest,
  parseCommitDetailInput,
  parseCommitFileDiffInput,
  parseCommitHistoryInput,
} from '@/api/history'
import { HttpError } from '@/api/client'
import { createServerFn } from '@tanstack/react-start'

export const loadCommitHistory = createServerFn({ method: 'GET' })
  .validator(parseCommitHistoryInput)
  .handler(({ data }) => loadCommitHistoryForRequest(data))

export const loadOptionalPrivateCommitHistory = createServerFn({ method: 'GET' })
  .validator(parseCommitHistoryInput)
  .handler(async ({ data }) => {
    try {
      return await loadCommitHistoryForRequest(data)
    } catch (error) {
      if (error instanceof HttpError && [403, 404].includes(error.status)) {
        return null
      }
      throw error
    }
  })

export const loadCommitDetail = createServerFn({ method: 'GET' })
  .validator(parseCommitDetailInput)
  .handler(({ data }) => loadCommitDetailForRequest(data))

export const loadCommitFileDiff = createServerFn({ method: 'GET' })
  .validator(parseCommitFileDiffInput)
  .handler(({ data }) => loadCommitFileDiffForRequest(data))
