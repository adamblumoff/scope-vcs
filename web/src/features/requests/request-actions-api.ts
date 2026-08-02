import { createApiClient } from '@/api/client'
import type { RequestParams } from '@/api/types'
import type {
  LeaveRequestResponse,
  RequestCloseResponse,
  RequestInviteeMutationResponse,
  RequestMutationResponse,
} from '@/api/types.generated'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'

export type RequestActionCommand =
  | { action: 'add_invitee'; handle: string }
  | { action: 'close' }
  | { action: 'leave' }
  | { action: 'merge' }
  | { action: 'ready' }
  | { action: 'remove_invitee'; handle: string }
  | { action: 'working' }

export type RequestActionInput = RequestParams & RequestActionCommand

export type RequestActionResult = {
  deleted: boolean
  synchronizationError?: string
}

export async function performRequestActionForRequest(
  input: RequestActionInput,
): Promise<RequestActionResult> {
  const api = createApiClient()
  const mutationOptions = { auth: 'required' as const }

  switch (input.action) {
    case 'ready':
      return mutationResult(await api.post<RequestMutationResponse>(
        requestRoute(ApiRouteTemplates.repoRequestReady, input),
        { ...mutationOptions, body: {} },
      ))
    case 'working':
      return mutationResult(await api.post<RequestMutationResponse>(
        requestRoute(ApiRouteTemplates.repoRequestWorking, input),
        mutationOptions,
      ))
    case 'merge':
      return mutationResult(await api.post<RequestMutationResponse>(
        requestRoute(ApiRouteTemplates.repoRequestMerge, input),
        mutationOptions,
      ))
    case 'close': {
      const result = await api.delete<RequestCloseResponse>(
        requestRoute(ApiRouteTemplates.repoRequest, input),
        mutationOptions,
      )
      return { deleted: result.deleted }
    }
    case 'add_invitee':
      return inviteeResult(await api.put<RequestInviteeMutationResponse>(
        requestRoute(ApiRouteTemplates.repoRequestInvitees, input),
        { ...mutationOptions, body: { handle: input.handle } },
      ))
    case 'remove_invitee':
      return inviteeResult(await api.delete<RequestInviteeMutationResponse>(
        requestRoute(ApiRouteTemplates.repoRequestInvitees, input),
        { ...mutationOptions, body: { handle: input.handle } },
      ))
    case 'leave':
      await api.delete<LeaveRequestResponse>(
        requestRoute(ApiRouteTemplates.repoRequestInviteesMe, input),
        mutationOptions,
      )
      return { deleted: false }
  }
}

function requestRoute(template: string, data: RequestParams) {
  return buildApiPath(template, {
    owner: data.owner,
    repo: data.repo,
    request_id: data.request_id,
  })
}

function mutationResult(_result: RequestMutationResponse): RequestActionResult {
  return { deleted: false }
}

function inviteeResult(_result: RequestInviteeMutationResponse): RequestActionResult {
  return { deleted: false }
}
