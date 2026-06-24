import {
  loadCommitDetailForRequest,
  loadCommitFileDiffForRequest,
  loadCommitHistoryForRequest,
  parseCommitDetailInput,
  parseCommitFileDiffInput,
  parseCommitHistoryInput,
} from '@/api/history'
import { createServerFn } from '@tanstack/react-start'

export const loadCommitHistory = createServerFn({ method: 'GET' })
  .validator(parseCommitHistoryInput)
  .handler(({ data }) => loadCommitHistoryForRequest(data))

export const loadCommitDetail = createServerFn({ method: 'GET' })
  .validator(parseCommitDetailInput)
  .handler(({ data }) => loadCommitDetailForRequest(data))

export const loadCommitFileDiff = createServerFn({ method: 'GET' })
  .validator(parseCommitFileDiffInput)
  .handler(({ data }) => loadCommitFileDiffForRequest(data))
