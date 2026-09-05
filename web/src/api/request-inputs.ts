import { parseRepoParams } from './repo-params'
import type { RequestParams } from './types'
import type { RequestActionInput } from '../features/requests/request-actions-api'
import type {
  CreateDiscussionInput, CreateReplyInput, LoadDiscussionsInput, LoadRepliesInput,
  MarkDiscussionReadInput, RequestDiscussionActionInput, UpdateDescriptionInput,
} from '../features/requests/request-discussion-api'

function object(input: unknown): Record<string, unknown> {
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    throw new Error('Input must be an object.')
  }
  return input as Record<string, unknown>
}

function text(value: unknown, field: string, maxBytes: number, allowEmpty = false): string {
  if (typeof value !== 'string' || (!allowEmpty && !value.trim())) {
    throw new Error(`${field} must be a string${allowEmpty ? '' : ' that is not empty'}.`)
  }
  if (new TextEncoder().encode(value).length > maxBytes) {
    throw new Error(`${field} exceeds ${maxBytes} bytes.`)
  }
  return value
}

function id(value: unknown, field: string): string {
  const result = text(value, field, 128).trim()
  if (/[\s/\\\u0000-\u001f\u007f]/u.test(result)) throw new Error(`${field} is invalid.`)
  return result
}

function optionalId(value: unknown, field: string): string | undefined {
  return value === undefined ? undefined : id(value, field)
}

function position(value: unknown, field: string, min = 0, max = Number.MAX_SAFE_INTEGER): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < min || value > max) {
    throw new Error(`${field} must be an integer between ${min} and ${max}.`)
  }
  return value
}

function filePath(value: unknown): string {
  const path = text(value, 'path', 4096)
  if (path.includes('\0')) throw new Error('path contains a NUL byte.')
  return path
}

export function parseRequestParams(input: unknown): RequestParams {
  const data = object(input)
  return { ...parseRepoParams(data), request_id: id(data.request_id, 'request_id') }
}

export function parseRepoFileInput(input: unknown) {
  const data = object(input)
  return { ...parseRepoParams(data), path: filePath(data.path) }
}

export function parseLoadRequestRevisionsInput(input: unknown) {
  const data = object(input)
  return {
    ...parseRequestParams(data),
    commit_oid: optionalId(data.commit_oid, 'commit_oid'),
    revision_id: optionalId(data.revision_id, 'revision_id'),
  }
}

export function parseLoadRequestRevisionDiffInput(input: unknown) {
  const data = object(input)
  return {
    ...parseRequestParams(data),
    commit_oid: id(data.commit_oid, 'commit_oid'),
    revision_id: id(data.revision_id, 'revision_id'),
    path: filePath(data.path),
  }
}

export function parseLoadDiscussionsInput(input: unknown): LoadDiscussionsInput {
  const data = object(input)
  if (data.include_revision_anchor !== undefined && typeof data.include_revision_anchor !== 'boolean') {
    throw new Error('include_revision_anchor must be a boolean.')
  }
  return {
    ...parseLoadRequestRevisionsInput(data),
    discussion_id: optionalId(data.discussion_id, 'discussion_id'),
    cursor: data.cursor === undefined ? undefined : text(data.cursor, 'cursor', 4096),
    include_revision_anchor: data.include_revision_anchor,
    limit: data.limit === undefined ? undefined : position(data.limit, 'limit', 1, 100),
  }
}

export function parseDiscussionActionInput(input: unknown): RequestDiscussionActionInput {
  const data = object(input)
  return { ...parseRequestParams(data), discussion_id: id(data.discussion_id, 'discussion_id') }
}

export function parseLoadRepliesInput(input: unknown): LoadRepliesInput {
  const data = object(input)
  return {
    ...parseDiscussionActionInput(data),
    before: data.before === undefined ? undefined : position(data.before, 'before'),
    reply: optionalId(data.reply, 'reply'),
  }
}

export function parseLoadDiscussionChangesInput(input: unknown) {
  const data = object(input)
  return { ...parseRequestParams(data), after: position(data.after, 'after') }
}

export function parseMarkDiscussionReadInput(input: unknown): MarkDiscussionReadInput {
  const data = object(input)
  return { ...parseDiscussionActionInput(data), through_position: position(data.through_position, 'through_position') }
}

export function parseCreateDiscussionInput(input: unknown): CreateDiscussionInput {
  const data = object(input)
  const anchor = data.anchor === null ? null : object(data.anchor)
  return {
    ...parseRequestParams(data),
    body_markdown: text(data.body_markdown, 'body_markdown', 64 * 1024),
    client_discussion_id: id(data.client_discussion_id, 'client_discussion_id'),
    anchor: anchor === null ? null : {
      revision_id: id(anchor.revision_id, 'revision_id'),
      commit_oid: anchor.commit_oid === null ? null : id(anchor.commit_oid, 'commit_oid'),
      path: anchor.path === null ? null : filePath(anchor.path),
    },
  }
}

export function parseCreateReplyInput(input: unknown): CreateReplyInput {
  const data = object(input)
  return {
    ...parseDiscussionActionInput(data),
    body_markdown: text(data.body_markdown, 'body_markdown', 64 * 1024),
    client_reply_id: id(data.client_reply_id, 'client_reply_id'),
    reply_to_reply_id: data.reply_to_reply_id === null ? null : id(data.reply_to_reply_id, 'reply_to_reply_id'),
  }
}

export function parseUpdateDescriptionInput(input: unknown): UpdateDescriptionInput {
  const data = object(input)
  return { ...parseRequestParams(data), description_markdown: text(data.description_markdown, 'description_markdown', 256 * 1024, true) }
}

export function parseRateRequestInput(input: unknown) {
  const data = object(input)
  return {
    ...parseRequestParams(data),
    score: position(data.score, 'score', 1, 5),
    reason: text(data.reason, 'reason', 1024),
  }
}

export function parseRequestActionInput(input: unknown): RequestActionInput {
  const data = object(input)
  const params = parseRequestParams(data)
  switch (data.action) {
    case 'add_invitee':
    case 'remove_invitee':
      return { ...params, action: data.action, handle: id(data.handle, 'handle') }
    case 'close':
    case 'leave':
    case 'merge':
    case 'submit':
      return { ...params, action: data.action }
    default:
      throw new Error('Unsupported request action.')
  }
}
