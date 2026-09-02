import { PageContent } from '@/components/page-header'
import { PendingSurface } from '@/components/pending-surface'
import { SectionRow, SectionRows } from '@/components/section-rows'
import {
  BlockSkeleton,
  TextSkeleton,
  type TextSkeletonLength,
} from '@/components/ui/skeleton'

const MEMBER_LABEL_LENGTHS: TextSkeletonLength[] = ['medium', 'short']

export function RepoSettingsPending() {
  return (
    <PendingSurface label="Loading repository settings">
      <PageContent>
        <h1 className="sr-only">Settings</h1>
        <SectionRows>
          {['danger', 'invite', 'members'].map((row) => (
            <SectionRow
              description={(
                <>
                  <TextSkeleton length="medium" size="meta" />
                  <TextSkeleton className="mt-1.5" length="short" size="meta" />
                </>
              )}
              key={row}
              title={<TextSkeleton length="short" />}
            >
              <div className="space-y-3">
                {row === 'danger' ? <BlockSkeleton className="h-8 w-40" /> : null}
                {row === 'invite' ? (
                  <>
                    <div className="flex gap-2">
                      <BlockSkeleton className="h-10 min-w-0 flex-1" />
                      <BlockSkeleton className="h-10 w-24 shrink-0" />
                    </div>
                    <BlockSkeleton className="h-16 w-full" />
                  </>
                ) : null}
                {row === 'members' ? (
                  <div className="divide-y divide-border border-y border-border">
                    {MEMBER_LABEL_LENGTHS.map((length) => (
                      <div
                        className="flex items-center justify-between gap-3 py-3"
                        key={length}
                      >
                        <div className="min-w-0 flex-1">
                          <TextSkeleton length={length} />
                          <TextSkeleton
                            className="mt-2"
                            length="medium"
                            size="meta"
                          />
                        </div>
                        <BlockSkeleton className="h-6 w-12 shrink-0" />
                      </div>
                    ))}
                  </div>
                ) : null}
              </div>
            </SectionRow>
          ))}
        </SectionRows>
      </PageContent>
    </PendingSurface>
  )
}
