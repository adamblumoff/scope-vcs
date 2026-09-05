import { createApiClient } from '@/api/client'
import type { ApiClient } from '@/api/client'
import type {
  RepoParams,
  RepoRunHistoryInput,
  RunActionInput,
  RunStepLogsInput,
} from '@/api/types'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'
import { apiValidators } from '@/api/validators.generated'

export async function loadRepoRunWorkflowsForRequest(
  data: RepoParams,
  api: ApiClient = createApiClient(),
) {
  return api.get(
    repoPath(ApiRouteTemplates.repoRunWorkflows, data),
    apiValidators.RepositoryRunWorkflowListResponse,
    { auth: 'optional' },
  )
}

export async function loadRepoRunHistoryForRequest(
  data: RepoRunHistoryInput,
  api: ApiClient = createApiClient(),
) {
  const query = new URLSearchParams()
  if (data.workflow) query.set('workflow', data.workflow)
  if (data.after) query.set('after', data.after)
  if (data.limit !== undefined) query.set('limit', data.limit.toString())
  const suffix = query.size ? `?${query}` : ''
  return api.get(
    `${repoPath(ApiRouteTemplates.repoRuns, data)}${suffix}`,
    apiValidators.RepositoryRunHistoryPageResponse,
    { auth: 'optional' },
  )
}

export async function loadRepoRunDetailForRequest(
  data: RunActionInput,
  api: ApiClient = createApiClient(),
) {
  return api.get(
    runPath(ApiRouteTemplates.repoRunDetail, data),
    apiValidators.RepositoryRunDetailResponse,
    { auth: 'required' },
  )
}

export async function loadRepoRunStepLogsForRequest(data: RunStepLogsInput) {
  const path = buildApiPath(ApiRouteTemplates.repoRunStepLogs, {
    attempt_id: data.attempt_id,
    owner: data.owner,
    repo: data.repo,
    run_id: data.run_id,
    step_index: data.step_index.toString(),
  })
  const query = new URLSearchParams()
  if (data.after !== undefined) query.set('after', data.after.toString())
  if (data.before !== undefined) query.set('before', data.before.toString())
  return createApiClient().get(
    `${path}${query.size ? `?${query}` : ''}`,
    apiValidators.RepositoryRunStepLogPageResponse,
    { auth: 'required' },
  )
}

export async function cancelRepoRunForRequest(data: RunActionInput) {
  await createApiClient().post(
    runPath(ApiRouteTemplates.repoRunCancel, data),
    apiValidators.RunResponse,
    { auth: 'required' },
  )
}

export async function retryRepoRunForRequest(data: RunActionInput) {
  await createApiClient().post(
    runPath(ApiRouteTemplates.repoRunRetry, data),
    apiValidators.RunResponse,
    { auth: 'required' },
  )
}

export function parseRunActionInput(data: RunActionInput): RunActionInput {
  return {
    ...parseRepoParams(data),
    run_id: requiredSegment('run_id', data.run_id),
  }
}

export function parseRepoRunHistoryInput(
  data: RepoRunHistoryInput,
): RepoRunHistoryInput {
  const input = parseRepoParams(data)
  const workflow = data.workflow === undefined
    ? undefined
    : requiredSegment('workflow', data.workflow)
  const after = data.after === undefined
    ? undefined
    : requiredValue('after', data.after)
  if (
    data.limit !== undefined &&
    (!Number.isSafeInteger(data.limit) || data.limit < 1 || data.limit > 100)
  ) {
    throw new Error('limit must be an integer between 1 and 100')
  }
  return { ...input, after, limit: data.limit, workflow }
}

export function parseRunStepLogsInput(data: RunStepLogsInput): RunStepLogsInput {
  const input = parseRunActionInput(data)
  const attemptId = requiredSegment('attempt_id', data.attempt_id)
  for (const name of ['after', 'before'] as const) {
    const cursor = data[name]
    if (cursor !== undefined && (!Number.isSafeInteger(cursor) || cursor < 0)) {
      throw new Error(`${name} must be a non-negative integer`)
    }
  }
  if (data.after !== undefined && data.before !== undefined) {
    throw new Error('choose after or before, not both')
  }
  if (!Number.isSafeInteger(data.step_index) || data.step_index < 0) {
    throw new Error('step_index must be a non-negative integer')
  }
  return {
    ...input,
    after: data.after,
    before: data.before,
    attempt_id: attemptId,
    step_index: data.step_index,
  }
}

function parseRepoParams(data: RepoParams): RepoParams {
  return {
    owner: requiredSegment('owner', data.owner),
    repo: requiredSegment('repo', data.repo),
  }
}

function requiredSegment(label: string, value: string) {
  const parsed = value.trim()
  if (!parsed || parsed.includes('/')) {
    throw new Error(`${label} must be one path segment`)
  }
  return parsed
}

function requiredValue(label: string, value: string) {
  const parsed = value.trim()
  if (!parsed) throw new Error(`${label} cannot be empty`)
  return parsed
}

function repoPath(template: string, data: RepoParams) {
  return buildApiPath(template, { owner: data.owner, repo: data.repo })
}

function runPath(template: string, data: RunActionInput) {
  return buildApiPath(template, {
    owner: data.owner,
    repo: data.repo,
    run_id: data.run_id,
  })
}
