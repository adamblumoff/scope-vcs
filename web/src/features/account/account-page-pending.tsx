import { ApplicationPendingShell } from '@/components/pending-surface'
import { SectionRow, SectionRows } from '@/components/section-rows'
import {
  BlockSkeleton,
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'

const SESSION_LABEL_LENGTHS: TextSkeletonLength[] = ['medium', 'long']

export function AccountPagePending() {
  return (
    <ApplicationPendingShell contextLabel="Account" label="Loading account">
      <div className="py-8 lg:py-10">
        <h1 className="text-[26px] font-semibold leading-[1.15] tracking-[-0.02em] sm:text-[32px]">
          Account
        </h1>
        <p className="mt-2 text-[15px] leading-6 text-muted-foreground">
          Manage Scope CLI access for this account.
        </p>
        <SectionRows>
          {['login', 'sessions'].map((row) => (
            <SectionRow
              description={(
                <>
                  <TextSkeleton length="medium" size="meta" />
                  <TextSkeleton className="mt-1.5" length="medium" size="meta" />
                </>
              )}
              key={row}
              title={<TextSkeleton length="short" />}
            >
              {row === 'login' ? (
                <BlockSkeleton className="h-8 w-32" />
              ) : (
                <div className="divide-y divide-border border-y border-border">
                  {SESSION_LABEL_LENGTHS.map((length) => (
                    <div
                      className="flex items-center justify-between gap-3 py-3"
                      key={length}
                    >
                      <div className="min-w-0 flex-1">
                        <TextSkeleton length={length} />
                        <TextSkeleton
                          className="mt-2"
                          length="long"
                          size="meta"
                        />
                      </div>
                      <BlockSkeleton className="size-8 shrink-0" />
                    </div>
                  ))}
                </div>
              )}
            </SectionRow>
          ))}
        </SectionRows>
      </div>
    </ApplicationPendingShell>
  )
}
