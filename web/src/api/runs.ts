import { createApiClient } from '@/api/client'
import type {
  RepoOperations,
  RepoParams,
  RepoRunDetail,
  RepoRunStepLogPage,
  RunActionInput,
  RunStepLogsInput,
} from '@/api/types'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'

export async function loadRepoOperationsForRequest(data: RepoParams) {
  return createApiClient().get<RepoOperations>(
    repoPath(ApiRouteTemplates.repoOperations, data),
    { auth: 'optional' },
  )
}

export async function loadRepoRunDetailForRequest(data: RunActionInput) {
  return createApiClient().get<RepoRunDetail>(
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
