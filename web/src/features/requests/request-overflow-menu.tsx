import { Button } from '@/components/ui/button'
import * as DropdownMenu from '@radix-ui/react-dropdown-menu'
import { Ellipsis, History } from 'lucide-react'

export function RequestOverflowMenu({
  onViewHistory,
}: {
  onViewHistory: () => void
}) {
  function viewHistoryAfterMenuCloses() {
    window.setTimeout(onViewHistory, 0)
  }

  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger asChild>
        <Button
          aria-label="More request actions"
          size="icon-sm"
          type="button"
          variant="secondary"
        >
          <Ellipsis className="size-4" />
        </Button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          align="end"
          className="z-50 min-w-48 rounded-lg border border-[var(--border-strong)] bg-popover p-1 text-popover-foreground shadow-[var(--shadow-pop)]"
          sideOffset={6}
        >
          <DropdownMenu.Item
            className="flex cursor-default select-none items-center gap-2 rounded-md px-2.5 py-2 text-sm outline-none data-[highlighted]:bg-muted"
            onSelect={viewHistoryAfterMenuCloses}
          >
            <History className="size-3.5 text-muted-foreground" />
            View request history
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  )
}
