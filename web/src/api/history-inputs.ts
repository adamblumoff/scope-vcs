import { parseRepoParams } from './repo-params'
import type {
  HistoryEntryDetailInput,
  HistoryEntryFileDiffInput,
  HistoryPageInput,
  ProjectionPreviewAudience,
} from './types'

export function parseHistoryPageInput(input: unknown): HistoryPageInput {
  return {
    ...parseRepoParams(input),
    audience: parseOptionalAudience(input),
    before: parseOptionalBefore(input),
  }
}

export function parseHistoryEntryDetailInput(input: unknown): HistoryEntryDetailInput {
  const data = input as Partial<HistoryEntryDetailInput> | null
  const entry = typeof data?.entry === 'string' ? data.entry.trim() : ''
  if (!entry) {
    throw new Error('A history entry id is required.')
  }

  return {
    ...parseRepoParams(input),
    audience: parseOptionalAudience(input),
    entry,
  }
}

export function parseHistoryEntryFileDiffInput(input: unknown): HistoryEntryFileDiffInput {
  const data = input as Partial<HistoryEntryFileDiffInput> | null
  const path = typeof data?.path === 'string' ? data.path.trim() : ''
  if (!path) {
    throw new Error('A file path is required.')
  }

  return {
    ...parseHistoryEntryDetailInput(input),
    path,
  }
}

export function parseHistoryAudience(
  audience: unknown,
): ProjectionPreviewAudience {
  if (audience === 'private' || audience === 'public') {
    return audience
  }
  throw new Error(`Unsupported history audience: ${String(audience)}`)
}

function parseOptionalAudience(input: unknown): ProjectionPreviewAudience | null {
  const audience = (input as { audience?: unknown } | null)?.audience
  if (audience === undefined || audience === null || audience === '') {
    return null
  }
  return parseHistoryAudience(audience)
}

function parseOptionalBefore(input: unknown) {
  const data = input as Partial<HistoryPageInput> | null
  if (typeof data?.before !== 'string') return null
  return data.before.trim() || null
}
