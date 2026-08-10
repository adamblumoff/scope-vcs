import type { RepoRunCache } from '@/api/types'
import {
  cacheExplanation,
  cacheNamespace,
  cacheStateClass,
  cacheStateLabel,
  cacheSummaryLabel,
  cacheTimingLabel,
  pinnedImageLabel,
} from './run-attempt-environment-model'

export function RunAttemptEnvironment({
  caches,
  pinnedContainerImage,
}: {
  caches: readonly RepoRunCache[]
  pinnedContainerImage: string | null
}) {
  return (
    <section aria-label="Execution environment" className="border-b border-border">
      <div className="grid gap-1 px-3 py-3 text-xs sm:grid-cols-[7rem_minmax(0,1fr)_auto] sm:items-baseline sm:gap-3">
        <strong className="text-sm font-medium">Environment</strong>
        <span className="text-muted-foreground">
          {cacheSummaryLabel(caches)}
        </span>
        <code
          className="text-[11px] text-muted-foreground"
          title={pinnedContainerImage ?? undefined}
        >
          {pinnedImageLabel(pinnedContainerImage)}
        </code>
      </div>
      {caches.length > 0 ? (
        <div className="divide-y divide-border/70 border-t border-border/70">
          {caches.map((cache) => {
            const state = cacheStateLabel(cache)
            return (
              <div
                className="grid gap-x-3 gap-y-1 px-3 py-2.5 text-xs sm:grid-cols-[7rem_6rem_minmax(0,1fr)_auto] sm:items-baseline"
                key={cache.name}
              >
                <strong className="font-mono font-medium">{cache.name}</strong>
                <span
                  className={`font-medium ${cacheStateClass(cache)}`}
                >
                  {state}
                </span>
                <span className="min-w-0">
                  <span className="block">{cacheExplanation(cache)}</span>
                  <code className="block truncate text-[10px] text-muted-foreground">
                    {cacheNamespace(cache)}
                  </code>
                </span>
                <span className="text-muted-foreground sm:text-right">
                  {cacheTimingLabel(cache)}
                </span>
              </div>
            )
          })}
        </div>
      ) : null}
    </section>
  )
}
