import type {
  ProjectionPreviewAudience,
  ProjectionPreviewSource,
} from '@/api/types'
import { Badge } from '@/components/ui/badge'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { Globe2, UserRound } from 'lucide-react'
import { audienceLabel, sourceLabel } from './review-labels'

export function ReviewVisibilityPanelHeader({
  audience,
  availableAudiences,
  description,
  onSelectAudience,
  source,
  title,
}: {
  audience: ProjectionPreviewAudience
  availableAudiences: ProjectionPreviewAudience[]
  description: string
  onSelectAudience: (audience: ProjectionPreviewAudience) => void
  source: ProjectionPreviewSource
  title: string
}) {
  return (
    <div className="flex flex-col gap-3 border-b border-border py-4 sm:flex-row sm:items-start sm:justify-between">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <h2 className="text-sm font-semibold leading-5">{title}</h2>
          <Badge variant="outline">{sourceLabel(source)}</Badge>
        </div>
        <p className="mt-1 max-w-[720px] text-sm leading-5 text-muted-foreground">
          {description}
        </p>
      </div>
      <div className="flex shrink-0 items-start sm:justify-end">
        {availableAudiences.length > 1 && (
          <AudienceToggle
            audience={audience}
            availableAudiences={availableAudiences}
            onSelect={onSelectAudience}
          />
        )}
      </div>
    </div>
  )
}

function AudienceToggle({
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
      {(['owner', 'public'] as const).map((option) => {
        const Icon = option === 'owner' ? UserRound : Globe2
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
