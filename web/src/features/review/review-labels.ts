import type { ProjectionPreviewAudience } from '@/api/types'

export function audienceLabel(audience: ProjectionPreviewAudience) {
  return audience === 'private' ? 'Private' : 'Public'
}

export function changeCountLabel(count: number) {
  return `${count} ${count === 1 ? 'change' : 'changes'}`
}
