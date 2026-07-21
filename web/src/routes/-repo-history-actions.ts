import {
  loadCommitDetailForRequest,
  loadCommitFileDiffForRequest,
  loadCommitHistoryForRequest,
  parseCommitDetailInput,
  parseCommitFileDiffInput,
  parseCommitHistoryInput,
} from '@/api/history'
import { HttpError } from '@/api/client'
import {
  loadRequestChangeBlockFileDiffForRequest,
  loadRequestChangeBlockFilesForRequest,
} from '@/api/requests'
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

export const loadRequestRevision = createServerFn({ method: 'GET' })
  .validator((data: RequestRevisionInput) => data)
  .handler(({ data }) => loadRequestChangeBlockFilesForRequest({
    block_id: data.revision,
    owner: data.owner,
    repo: data.repo,
    request_id: data.request,
  }))

export const loadRequestRevisionFileDiff = createServerFn({ method: 'GET' })
  .validator((data: RequestRevisionFileInput) => data)
  .handler(({ data }) => loadRequestChangeBlockFileDiffForRequest({
    block_id: data.revision,
    owner: data.owner,
    path: data.path,
    repo: data.repo,
    request_id: data.request,
  }))

type RequestRevisionInput = {
  owner: string
  repo: string
  request: string
  revision: string
}

type RequestRevisionFileInput = RequestRevisionInput & { path: string }
