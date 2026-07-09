import type {
  AddRequestEditorInput,
  CommentRequestInput,
  DeleteRequestInput,
  MergeRequestInput,
  NeedsResponseInput,
  RepoParams,
  RequestDelete,
  RequestDetail,
  RequestMutation,
  RequestWorkflowDisposition,
  RemoveRequestEditorInput,
  ResolveRequestInput,
  RespondRequestInput,
} from '@/api/types'
import { useRouter } from '@tanstack/react-router'
import { type FormEvent, useReducer, useState } from 'react'
import {
  normalizedBody,
  resolutionOptionsFor,
} from './request-labels'

type RequestMutationAction<TInput> = (input: TInput) => Promise<RequestMutation>
type RequestDeleteAction = (input: DeleteRequestInput) => Promise<RequestDelete>

export type RequestActionKey =
  | 'add-editor'
  | 'comment'
  | 'delete'
  | 'merge'
  | 'needs-response'
  | 'remove-editor'
  | 'resolve'
  | 'respond'

export type RequestActionError = {
  key: RequestActionKey
  message: string
}

type RequestBodyField =
  | 'commentBody'
  | 'editorUserId'
  | 'needsResponseBody'
  | 'resolveBody'
  | 'responseBody'

type RequestDetailUiState = {
  actionError: RequestActionError | null
  commentBody: string
  editorUserId: string
  mergeOpen: boolean
  needsResponseBody: string
  pendingAction: RequestActionKey | null
  resolveBody: string
  resolveDisposition: RequestWorkflowDisposition
  responseBody: string
}

type RequestDetailUiAction =
  | { field: RequestBodyField; type: 'bodyChanged'; value: string }
  | { disposition: RequestWorkflowDisposition; type: 'resolveDispositionChanged' }
  | { open: boolean; type: 'mergeOpenChanged' }
  | { key: RequestActionKey; type: 'actionStarted' }
  | {
      closeMerge?: boolean
      resetField?: RequestBodyField
      type: 'actionSucceeded'
    }
  | { key: RequestActionKey; message: string; type: 'actionFailed' }

const initialRequestDetailUiState: RequestDetailUiState = {
  actionError: null,
  commentBody: '',
  editorUserId: '',
  mergeOpen: false,
  needsResponseBody: '',
  pendingAction: null,
  resolveBody: '',
  resolveDisposition: 'UsefulNotMerged',
  responseBody: '',
}

export type RequestDetailControllerProps = {
  addRequestEditor: RequestMutationAction<AddRequestEditorInput>
  commentRequest: RequestMutationAction<CommentRequestInput>
  deleteRequest: RequestDeleteAction
  detail: RequestDetail
  markNeedsResponse: RequestMutationAction<NeedsResponseInput>
  mergeRequest: RequestMutationAction<MergeRequestInput>
  params: RepoParams
  removeRequestEditor: RequestMutationAction<RemoveRequestEditorInput>
  resolveRequest: RequestMutationAction<ResolveRequestInput>
  respondToRequest: RequestMutationAction<RespondRequestInput>
}

export function useRequestDetailController({
  addRequestEditor,
  commentRequest,
  deleteRequest,
  detail,
  markNeedsResponse,
  mergeRequest,
  params,
  removeRequestEditor,
  resolveRequest,
  respondToRequest,
}: RequestDetailControllerProps) {
  const router = useRouter()
  const { request } = detail
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [uiState, dispatch] = useReducer(
    requestDetailUiReducer,
    initialRequestDetailUiState,
  )
  const resolutionOptions = resolutionOptionsFor(request)
  const activeResolveDisposition = resolutionOptions.some(
    (option) => option.disposition === uiState.resolveDisposition,
  )
    ? uiState.resolveDisposition
    : resolutionOptions[0]?.disposition ?? 'UsefulNotMerged'
  const requestParams = { ...params, request_id: request.id }

  async function runAction(
    key: RequestActionKey,
    action: () => Promise<unknown>,
    success?: { closeMerge?: boolean; resetField?: RequestBodyField },
  ) {
    dispatch({ key, type: 'actionStarted' })
    try {
      await action()
      await router.invalidate()
      dispatch({ type: 'actionSucceeded', ...success })
    } catch (error) {
      dispatch({
        key,
        message: error instanceof Error ? error.message : 'request action failed',
        type: 'actionFailed',
      })
    }
  }

  async function submitComment(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'comment',
      () => commentRequest({ ...requestParams, body: uiState.commentBody }),
      { resetField: 'commentBody' },
    )
  }

  async function submitNeedsResponse(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'needs-response',
      () =>
        markNeedsResponse({
          ...requestParams,
          body: uiState.needsResponseBody,
        }),
      { resetField: 'needsResponseBody' },
    )
  }

  async function submitResponse(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'respond',
      () =>
        respondToRequest({
          ...requestParams,
          body: normalizedBody(uiState.responseBody),
        }),
      { resetField: 'responseBody' },
    )
  }

  async function submitResolution(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'resolve',
      () =>
        resolveRequest({
          ...requestParams,
          body: normalizedBody(uiState.resolveBody),
          disposition: activeResolveDisposition,
        }),
      { resetField: 'resolveBody' },
    )
  }

  async function submitMerge(body: string | null) {
    const currentMainOid = request.mergeability.current_main_oid
    if (!currentMainOid) {
      dispatch({
        key: 'merge',
        message: 'Request has no current main OID to merge into.',
        type: 'actionFailed',
      })
      return
    }

    await runAction(
      'merge',
      () =>
        mergeRequest({
          ...requestParams,
          body,
          expected_head_oid: request.mergeability.request_head_oid,
          expected_main_oid: currentMainOid,
        }),
      { closeMerge: true },
    )
  }

  async function submitDelete() {
    dispatch({ key: 'delete', type: 'actionStarted' })
    try {
      const result = await deleteRequest(requestParams)
      if (result.deleted) {
        await router.navigate({
          params,
          to: '/repos/$owner/$repo/requests',
        })
        return
      }
      await router.invalidate()
      setDeleteOpen(false)
      dispatch({ type: 'actionSucceeded' })
    } catch (error) {
      dispatch({
        key: 'delete',
        message:
          error instanceof Error ? error.message : 'request delete failed',
        type: 'actionFailed',
      })
    }
  }

  async function submitAddEditor(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'add-editor',
      () =>
        addRequestEditor({
          ...requestParams,
          user_id: uiState.editorUserId,
        }),
      { resetField: 'editorUserId' },
    )
  }

  async function removeEditor(editorUserId: string) {
    await runAction('remove-editor', () =>
      removeRequestEditor({
        ...requestParams,
        editor_user_id: editorUserId,
      }),
    )
  }

  return {
    activeResolveDisposition,
    deleteOpen,
    dispatch,
    removeEditor,
    request,
    resolutionOptions,
    setDeleteOpen,
    submitAddEditor,
    submitComment,
    submitDelete,
    submitMerge,
    submitNeedsResponse,
    submitResolution,
    submitResponse,
    uiState,
  }
}

function requestDetailUiReducer(
  state: RequestDetailUiState,
  action: RequestDetailUiAction,
): RequestDetailUiState {
  switch (action.type) {
    case 'bodyChanged':
      return { ...state, [action.field]: action.value }
    case 'resolveDispositionChanged':
      return { ...state, resolveDisposition: action.disposition }
    case 'mergeOpenChanged':
      return { ...state, mergeOpen: action.open }
    case 'actionStarted':
      return { ...state, actionError: null, pendingAction: action.key }
    case 'actionSucceeded':
      return {
        ...state,
        ...(action.resetField ? { [action.resetField]: '' } : {}),
        actionError: null,
        mergeOpen: action.closeMerge ? false : state.mergeOpen,
        pendingAction: null,
      }
    case 'actionFailed':
      return {
        ...state,
        actionError: { key: action.key, message: action.message },
        pendingAction: null,
      }
  }
}
