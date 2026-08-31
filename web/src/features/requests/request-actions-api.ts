import { createApiClient } from '@/api/client'
import type { RequestParams } from '@/api/types'
import type {
  RequestInviteeMutationResponse,
  RequestMutationResponse,
} from '@/api/types.generated'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'
import { apiValidators } from '@/api/validators.generated'

export type RequestActionCommand =
  | { action: 'add_invitee'; handle: string }
  | { action: 'close' }
  | { action: 'leave' }
  | { action: 'merge' }
  | { action: 'submit' }
  | { action: 'remove_invitee'; handle: string }

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
    case 'submit':
      return mutationResult(await api.post(
        requestRoute(ApiRouteTemplates.repoRequestSubmit, input),
        apiValidators.RequestMutationResponse,
        { ...mutationOptions, body: {} },
      ))
    case 'merge':
      return mutationResult(await api.post(
        requestRoute(ApiRouteTemplates.repoRequestMerge, input),
        apiValidators.RequestMutationResponse,
        mutationOptions,
      ))
    case 'close': {
      const result = await api.delete(
        requestRoute(ApiRouteTemplates.repoRequest, input),
        apiValidators.RequestCloseResponse,
        mutationOptions,
      )
      return { deleted: result.deleted }
    }
    case 'add_invitee':
      return inviteeResult(await api.put(
        requestRoute(ApiRouteTemplates.repoRequestInvitees, input),
        apiValidators.RequestInviteeMutationResponse,
        { ...mutationOptions, body: { handle: input.handle } },
      ))
    case 'remove_invitee':
      return inviteeResult(await api.delete(
        requestRoute(ApiRouteTemplates.repoRequestInvitees, input),
        apiValidators.RequestInviteeMutationResponse,
        { ...mutationOptions, body: { handle: input.handle } },
      ))
    case 'leave':
      await api.delete(
        requestRoute(ApiRouteTemplates.repoRequestInviteesMe, input),
        apiValidators.LeaveRequestResponse,
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
