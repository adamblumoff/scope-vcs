import { createApiClient, HttpError } from '@/api/client'
import {
  loadRepoRunDetailForRequest,
  loadRepoRunHistoryForRequest,
  loadRepoRunWorkflowsForRequest,
  parseRunActionInput,
  parseRepoRunHistoryInput,
} from '@/api/runs'
import { createServerFn } from '@tanstack/react-start'

export const loadRepoRunPage = createServerFn({ method: 'GET' })
  .validator(parseRepoRunHistoryInput)
  .handler(async ({ data }) => {
    try {
      const api = createApiClient()
      const [history, workflowResource] = await Promise.all([
        loadRepoRunHistoryForRequest(data, api),
        loadRepoRunWorkflowsForRequest(data, api)
          .then((workflows) => ({ error: null, workflows }))
          .catch((error: unknown) => ({
            error: resourceErrorMessage(error),
            workflows: { workflows: [] },
          })),
      ])
      const latest = history.runs[0]
        ? await loadRepoRunDetailForRequest(
            { ...data, run_id: history.runs[0].id },
            api,
          )
        : null
      return {
        history,
        latest,
        workflows: workflowResource.workflows,
        workflowsError: workflowResource.error,
      }
    } catch (error) {
      if (error instanceof HttpError && [401, 403, 404].includes(error.status)) {
        return null
      }
      throw error
    }
  })

function resourceErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Workflow catalog unavailable.'
}

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

export const loadRepoRunDetail = createServerFn({ method: 'GET' })
  .validator(parseRunActionInput)
  .handler(({ data }) => loadRepoRunDetailForRequest(data))
