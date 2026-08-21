import type { ProjectionPreviewAudience } from '@/api/types'

export function audienceLabel(audience: ProjectionPreviewAudience) {
  return audience === 'private' ? 'Private' : 'Public'
}
