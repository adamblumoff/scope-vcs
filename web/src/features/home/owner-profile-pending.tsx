import { ApplicationPendingShell } from '@/components/pending-surface'
import {
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'

const PENDING_REPOSITORIES: { id: string; length: TextSkeletonLength }[] = [
  { id: 'first', length: 'medium' },
  { id: 'second', length: 'short' },
  { id: 'third', length: 'long' },
  { id: 'fourth', length: 'medium' },
]

export function OwnerProfilePending({ owner }: { owner: string }) {
  return (
    <ApplicationPendingShell label={`Loading @${owner}`}>
      <div className="py-8 lg:py-10">
        <h1 className="text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[32px]">
          @{owner}
        </h1>
        <div className="mt-6 divide-y divide-border border-y border-border">
          {PENDING_REPOSITORIES.map((repository) => (
            <div className="py-4" key={repository.id}>
              <TextSkeleton length={repository.length} size="title" />
              <TextSkeleton className="mt-2" length="short" size="meta" />
            </div>
          ))}
        </div>
      </div>
    </ApplicationPendingShell>
  )
}
