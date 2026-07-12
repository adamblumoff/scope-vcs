import { cn } from '@/lib/utils'
import { Search } from 'lucide-react'
import type { ComponentProps } from 'react'

/** Visual contract for a future search surface. It is intentionally not mounted. */
function SearchControl({ className, ...props }: ComponentProps<'input'>) {
  return (
    <label className={cn('relative block w-64', className)}>
      <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
      <input
        className="h-10 w-full rounded-lg border border-input bg-secondary pl-9 pr-12 text-sm text-foreground shadow-[var(--shadow-card)] placeholder:text-muted-foreground focus-visible:border-ring focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        placeholder="Search"
        type="search"
        {...props}
      />
      <kbd className="pointer-events-none absolute right-2.5 top-1/2 -translate-y-1/2 rounded border border-border bg-background px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
        ⌘K
      </kbd>
    </label>
  )
}

export { SearchControl }
