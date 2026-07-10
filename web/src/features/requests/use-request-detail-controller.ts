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
import { type FormEvent, useState } from 'react'
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
  const [actionError, setActionError] = useState<RequestActionError | null>(null)
  const [commentBody, setCommentBody] = useState('')
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [editorUserId, setEditorUserId] = useState('')
  const [mergeOpen, setMergeOpen] = useState(false)
  const [needsResponseBody, setNeedsResponseBody] = useState('')
  const [pendingAction, setPendingAction] = useState<RequestActionKey | null>(null)
  const [resolveBody, setResolveBody] = useState('')
  const [resolveDisposition, setResolveDisposition] =
    useState<RequestWorkflowDisposition>('UsefulNotMerged')
  const [responseBody, setResponseBody] = useState('')
  const resolutionOptions = resolutionOptionsFor(request)
  const activeResolveDisposition = resolutionOptions.some(
    (option) => option.disposition === resolveDisposition,
  )
    ? resolveDisposition
    : resolutionOptions[0]?.disposition ?? 'UsefulNotMerged'
  const requestParams = { ...params, request_id: request.id }

  async function runAction(
    key: RequestActionKey,
    action: () => Promise<unknown>,
    onSuccess?: () => void,
  ) {
    setActionError(null)
    setPendingAction(key)
    try {
      await action()
      await router.invalidate()
      onSuccess?.()
    } catch (error) {
      setActionError({
        key,
        message: error instanceof Error ? error.message : 'request action failed',
      })
    } finally {
      setPendingAction(null)
    }
  }

  async function submitComment(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'comment',
      () => commentRequest({ ...requestParams, body: commentBody }),
      () => setCommentBody(''),
    )
  }

  async function submitNeedsResponse(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'needs-response',
      () =>
        markNeedsResponse({
          ...requestParams,
          body: needsResponseBody,
        }),
      () => setNeedsResponseBody(''),
    )
  }

  async function submitResponse(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'respond',
      () =>
        respondToRequest({
          ...requestParams,
          body: normalizedBody(responseBody),
        }),
      () => setResponseBody(''),
    )
  }

  async function submitResolution(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'resolve',
      () =>
        resolveRequest({
          ...requestParams,
          body: normalizedBody(resolveBody),
          disposition: activeResolveDisposition,
        }),
      () => setResolveBody(''),
    )
  }

  async function submitMerge(body: string | null) {
    const currentMainOid = request.mergeability.current_main_oid
    if (!currentMainOid) {
      setActionError({
        key: 'merge',
        message: 'Request has no current main OID to merge into.',
      })
      setPendingAction(null)
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
      () => setMergeOpen(false),
    )
  }

  async function submitDelete() {
    setActionError(null)
    setPendingAction('delete')
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
    } catch (error) {
      setActionError({
        key: 'delete',
        message:
          error instanceof Error ? error.message : 'request delete failed',
      })
    } finally {
      setPendingAction(null)
    }
  }

  async function submitAddEditor(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    await runAction(
      'add-editor',
      () =>
        addRequestEditor({
          ...requestParams,
          user_id: editorUserId,
        }),
      () => setEditorUserId(''),
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
    removeEditor,
    request,
    resolutionOptions,
    setCommentBody,
    setDeleteOpen,
    setEditorUserId,
    setMergeOpen,
    setNeedsResponseBody,
    setResolveBody,
    setResolveDisposition,
    setResponseBody,
    submitAddEditor,
    submitComment,
    submitDelete,
    submitMerge,
    submitNeedsResponse,
    submitResolution,
    submitResponse,
    uiState: {
      actionError,
      commentBody,
      editorUserId,
      mergeOpen,
      needsResponseBody,
      pendingAction,
      resolveBody,
      responseBody,
    },
  }
}
