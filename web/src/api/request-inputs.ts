import { parseRepoParams } from './repo-params'
import type {
  AddRequestEditorInput,
  CommentRequestInput,
  MergeRequestInput,
  NeedsResponseInput,
  RequestParams,
  RequestWorkflowDisposition,
  ResolveRequestInput,
  RespondRequestInput,
  RemoveRequestEditorInput,
} from './types'

const RESOLVE_DISPOSITIONS = new Set<RequestWorkflowDisposition>([
  'UsefulNotMerged',
  'HiddenContext',
  'NotAligned',
  'Duplicate',
  'Abandoned',
  'LowQuality',
])

export function parseRequestParams(input: unknown): RequestParams {
  const data = input as Partial<RequestParams> | null
  const requestId =
    typeof data?.request_id === 'string' ? data.request_id.trim() : ''

  if (!requestId) {
    throw new Error('Request route is incomplete.')
  }

  return {
    ...parseRepoParams(input),
    request_id: requestId,
  }
}

export function parseCommentRequestInput(input: unknown): CommentRequestInput {
  return {
    ...parseRequestParams(input),
    body: parseRequiredBody(input, 'Comment body is required.'),
  }
}

export function parseNeedsResponseInput(input: unknown): NeedsResponseInput {
  return {
    ...parseRequestParams(input),
    body: parseRequiredBody(input, 'Needs-response body is required.'),
  }
}

export function parseRespondRequestInput(input: unknown): RespondRequestInput {
  return {
    ...parseRequestParams(input),
    body: parseOptionalBody(input),
  }
}

export function parseResolveRequestInput(input: unknown): ResolveRequestInput {
  const data = input as Partial<ResolveRequestInput> | null
  const disposition = data?.disposition

  if (!RESOLVE_DISPOSITIONS.has(disposition as RequestWorkflowDisposition)) {
    throw new Error('Unsupported request disposition.')
  }

  return {
    ...parseRequestParams(input),
    body: parseOptionalBody(input),
    disposition: disposition as RequestWorkflowDisposition,
  }
}

export function parseMergeRequestInput(input: unknown): MergeRequestInput {
  const data = input as Partial<MergeRequestInput> | null
  const expectedMainOid = requiredString(
    data?.expected_main_oid,
    'Expected main OID is required.',
  )
  const expectedHeadOid = requiredString(
    data?.expected_head_oid,
    'Expected request head OID is required.',
  )

  return {
    ...parseRequestParams(input),
    body: parseOptionalBody(input),
    expected_head_oid: expectedHeadOid,
    expected_main_oid: expectedMainOid,
  }
}

export function parseAddRequestEditorInput(
  input: unknown,
): AddRequestEditorInput {
  const data = input as Partial<AddRequestEditorInput> | null
  return {
    ...parseRequestParams(input),
    user_id: requiredString(data?.user_id, 'Editor user id is required.'),
  }
}

export function parseRemoveRequestEditorInput(
  input: unknown,
): RemoveRequestEditorInput {
  const data = input as Partial<RemoveRequestEditorInput> | null
  return {
    ...parseRequestParams(input),
    editor_user_id: requiredString(
      data?.editor_user_id,
      'Editor user id is required.',
    ),
  }
}

function parseRequiredBody(input: unknown, message: string) {
  const body = parseOptionalBody(input)
  if (!body) {
    throw new Error(message)
  }
  return body
}

function parseOptionalBody(input: unknown) {
  const data = input as { body?: unknown } | null
  if (typeof data?.body !== 'string') {
    return null
  }
  const body = data.body.trim()
  return body ? body : null
}

function requiredString(input: unknown, message: string) {
  const value = typeof input === 'string' ? input.trim() : ''
  if (!value) {
    throw new Error(message)
  }
  return value
}
