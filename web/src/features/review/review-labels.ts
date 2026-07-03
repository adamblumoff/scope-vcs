import type {
  ProjectionPreviewAudience,
  ProjectionPreviewSource,
} from '@/api/types'

export function audienceLabel(audience: ProjectionPreviewAudience) {
  return audience === 'private' ? 'Private' : 'Public'
}

export function sourceLabel(source: ProjectionPreviewSource) {
  return source === 'review' ? 'After review' : 'Current repo'
}

export function fileCountLabel(count: number) {
  return `${count} ${count === 1 ? 'file' : 'files'}`
}

export function commitCountLabel(count: number) {
  return `${count} ${count === 1 ? 'commit' : 'commits'}`
}

export function changeCountLabel(count: number) {
  return `${count} ${count === 1 ? 'change' : 'changes'}`
}

export function olderCommitLabel(count: number) {
  return `Show ${count} older ${count === 1 ? 'commit' : 'commits'}`
}
