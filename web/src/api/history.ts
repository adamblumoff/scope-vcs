import { createApiClient } from '@/api/client'
import { parseHistoryAudience } from './history-inputs'
import type {
  HistoryEntryDetail,
  HistoryEntryDetailInput,
  HistoryEntryFileDiffInput,
  HistoryPage,
  HistoryPageInput,
  ReviewFileDiff,
} from './types'
import { ApiRouteTemplates, buildApiPath } from './types.generated'
export {
  parseHistoryEntryDetailInput,
  parseHistoryEntryFileDiffInput,
  parseHistoryPageInput,
} from './history-inputs'

export async function loadHistoryPageForRequest(
  data: HistoryPageInput,
): Promise<HistoryPage> {
  const query = new URLSearchParams()
  if (data.audience) query.set('audience', parseHistoryAudience(data.audience))
  if (data.before) query.set('before', data.before)

  return createApiClient().get<HistoryPage>(
    `${buildApiPath(ApiRouteTemplates.repoHistory, {
      owner: data.owner,
      repo: data.repo,
    })}?${query}`,
    { auth: 'optional' },
  )
}

export async function loadHistoryEntryForRequest(
  data: HistoryEntryDetailInput,
): Promise<HistoryEntryDetail> {
  const query = new URLSearchParams()
  if (data.audience) query.set('audience', parseHistoryAudience(data.audience))

  return createApiClient().get<HistoryEntryDetail>(
    `${buildApiPath(ApiRouteTemplates.repoHistoryEntry, {
      owner: data.owner,
      repo: data.repo,
      entry_id: data.entry,
    })}?${query}`,
    { auth: 'optional' },
  )
}

export async function loadHistoryEntryFileDiffForRequest(
  data: HistoryEntryFileDiffInput,
): Promise<ReviewFileDiff> {
  const query = new URLSearchParams({
    audience: parseHistoryAudience(data.audience),
    path: data.path,
  })

  return createApiClient().get<ReviewFileDiff>(
    `${buildApiPath(ApiRouteTemplates.repoHistoryEntryFileDiff, {
      owner: data.owner,
      repo: data.repo,
      entry_id: data.entry,
    })}?${query}`,
    { auth: 'optional' },
  )
}
