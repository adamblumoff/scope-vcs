import type { ProjectionPreviewAudience } from '@/api/types'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { Globe2, LockKeyhole } from 'lucide-react'
import { audienceLabel } from '../review/review-labels'

export function AudienceToggle({
  audience,
  availableAudiences,
  onSelect,
}: {
  audience: ProjectionPreviewAudience
  availableAudiences: ProjectionPreviewAudience[]
  onSelect: (audience: ProjectionPreviewAudience) => void
}) {
  return (
    <ToggleGroup
      onValueChange={(value) => {
        if (value) {
          onSelect(value as ProjectionPreviewAudience)
        }
      }}
      type="single"
      value={audience}
    >
      {(['private', 'public'] as const).map((option) => {
        const Icon = option === 'private' ? LockKeyhole : Globe2
        return (
          <ToggleGroupItem
            aria-label={`${audienceLabel(option)} view`}
            disabled={!availableAudiences.includes(option)}
            key={option}
            value={option}
          >
            <Icon className="size-3" />
            <span>{audienceLabel(option)} view</span>
          </ToggleGroupItem>
        )
      })}
    </ToggleGroup>
  )
}
