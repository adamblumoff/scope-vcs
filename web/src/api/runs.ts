import { createApiClient } from '@/api/client'
import type { ApiClient } from '@/api/client'
import type {
  RepoParams,
  RepoRunDetail,
  RepoRunHistoryInput,
  RepoRunHistoryPage,
  RepoRunStepLogPage,
  RepoRunWorkflowList,
  RunActionInput,
  RunStepLogsInput,
} from '@/api/types'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'

export async function loadRepoRunWorkflowsForRequest(
  data: RepoParams,
  api: ApiClient = createApiClient(),
) {
  return api.get<RepoRunWorkflowList>(
    repoPath(ApiRouteTemplates.repoRunWorkflows, data),
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
  return api.get<RepoRunHistoryPage>(
    `${repoPath(ApiRouteTemplates.repoRuns, data)}${suffix}`,
    { auth: 'optional' },
  )
}

export async function loadRepoRunDetailForRequest(
  data: RunActionInput,
  api: ApiClient = createApiClient(),
) {
  return api.get<RepoRunDetail>(
    runPath(ApiRouteTemplates.repoRunDetail, data),
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
  return createApiClient().get<RepoRunStepLogPage>(
    `${path}?after=${encodeURIComponent(data.after)}`,
    { auth: 'required' },
  )
}

export async function cancelRepoRunForRequest(data: RunActionInput) {
  await createApiClient().post(
    runPath(ApiRouteTemplates.repoRunCancel, data),
    { auth: 'required' },
  )
}

export async function retryRepoRunForRequest(data: RunActionInput) {
  await createApiClient().post(
    runPath(ApiRouteTemplates.repoRunRetry, data),
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
  if (!Number.isSafeInteger(data.after) || data.after < 0) {
    throw new Error('after must be a non-negative integer')
  }
  if (!Number.isSafeInteger(data.step_index) || data.step_index < 0) {
    throw new Error('step_index must be a non-negative integer')
  }
  return {
    ...input,
    after: data.after,
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
