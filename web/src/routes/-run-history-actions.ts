import { createApiClient, HttpError } from '@/api/client'
import {
  loadRepoRunHistoryForRequest,
  loadRepoRunWorkflowsForRequest,
  parseRepoRunHistoryInput,
} from '@/api/runs'
import { createServerFn } from '@tanstack/react-start'

export const loadRepoRunPage = createServerFn({ method: 'GET' })
  .validator(parseRepoRunHistoryInput)
  .handler(async ({ data }) => {
    try {
      const api = createApiClient()
      const [history, workflows] = await Promise.all([
        loadRepoRunHistoryForRequest(data, api),
        loadRepoRunWorkflowsForRequest(data, api),
      ])
      return { history, workflows }
    } catch (error) {
      if (error instanceof HttpError && [401, 403, 404].includes(error.status)) {
        return null
      }
      throw error
    }
  })

export const loadRepoRunHistory = createServerFn({ method: 'GET' })
  .validator(parseRepoRunHistoryInput)
  .handler(async ({ data }) => {
    try {
      return await loadRepoRunHistoryForRequest(data)
    } catch (error) {
      if (error instanceof HttpError && [401, 403, 404].includes(error.status)) {
        return null
      }
      throw error
    }
  })
