import { SectionRow, SectionRows } from '@/components/section-rows'
import { Button } from '@/components/ui/button'
import { Trash2 } from 'lucide-react'

export function SettingsSections({
  onDeleteRepository,
}: {
  onDeleteRepository: () => void
}) {
  return (
    <SectionRows>
      <SectionRow
        description="Permanently removes repo metadata and stored Git data from Scope."
        icon={<Trash2 className="size-4" />}
        title="Danger zone"
      >
        <Button
          onClick={onDeleteRepository}
          size="sm"
          type="button"
          variant="destructive"
        >
          <Trash2 className="size-3.5" />
          <span>Delete repository</span>
        </Button>
      </SectionRow>
    </SectionRows>
  )
}
