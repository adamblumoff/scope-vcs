import {
  loadHistoryEntryFileDiffForRequest,
  loadHistoryEntryForRequest,
  loadHistoryPageForRequest,
  parseHistoryEntryDetailInput,
  parseHistoryEntryFileDiffInput,
  parseHistoryPageInput,
} from '@/api/history'
import { createServerFn } from '@tanstack/react-start'

export const loadHistoryPage = createServerFn({ method: 'GET' })
  .validator(parseHistoryPageInput)
  .handler(({ data }) => loadHistoryPageForRequest(data))

export const loadHistoryEntry = createServerFn({ method: 'GET' })
  .validator(parseHistoryEntryDetailInput)
  .handler(({ data }) => loadHistoryEntryForRequest(data))

export const loadHistoryEntryFileDiff = createServerFn({ method: 'GET' })
  .validator(parseHistoryEntryFileDiffInput)
  .handler(({ data }) => loadHistoryEntryFileDiffForRequest(data))
