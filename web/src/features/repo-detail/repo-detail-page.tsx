import type { RepoDetail, RepoFile, RepoParams } from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { RepoBreadcrumb } from '@/components/repo-breadcrumb'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { RouteErrorPage } from '@/components/route-error-page'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { VisibilityBadge } from '@/components/visibility-badge'
import { Link } from '@tanstack/react-router'
import { File, Settings } from 'lucide-react'
import { RepoCloneDropdown } from './repo-clone-dropdown'

export function RepoDetailPage({
  detail,
  params,
}: {
  detail: RepoDetail
  params: RepoParams
}) {
  const { repo } = detail
  const files = detail.files
  const publicOnlyView = repo.access.actor === 'Public'

  return (
    <main className="min-h-screen bg-background text-foreground">
      <AppHeader breadcrumb={() => <RepoBreadcrumb params={params} />} />

      <PageContent>
        <PageHeader
          actions={() => (
            <>
              {repo.lifecycle_state === 'Published' && (
                <RepoCloneDropdown
                  cloneRemoteUrl={detail.clone_remote_url}
                  repo={repo}
                />
              )}
              <RepoPrimaryActionButton
                includeOpen={false}
                repo={repo}
                requireOwner
                variant="default"
              />
              {repo.access.actor !== 'Public' && (
                <Button asChild size="sm" variant="secondary">
                  <Link
                    params={{ owner: repo.owner_handle, repo: repo.name }}
                    to="/repos/$owner/$repo/settings"
                  >
                    <Settings className="size-3.5" />
                    <span>Settings</span>
                  </Link>
                </Button>
              )}
            </>
          )}
          badges={() => (
            <>
              <LifecycleBadge state={repo.lifecycle_state} />
              <Badge variant="neutral">{files.length} files</Badge>
            </>
          )}
          title={repo.id}
          titleClassName="font-mono"
        />

        <RepoFileList
          emptyTitle={publicOnlyView ? 'No public files' : 'No files'}
          files={files}
        />
      </PageContent>
    </main>
  )
}

function RepoFileList({
  emptyTitle,
  files,
}: {
  emptyTitle: string
  files: RepoFile[]
}) {
  return (
    <section className="mt-8">
      <div className="flex items-center justify-between gap-3 border-b border-border pb-3">
        <h2 className="text-base font-semibold text-balance">Files</h2>
        <Badge variant="neutral">{files.length}</Badge>
      </div>

      {files.length === 0 ? (
        <div className="border-b border-border py-10">
          <div className="text-sm font-medium">{emptyTitle}</div>
        </div>
      ) : (
        <div className="divide-y divide-border border-b border-border">
          <div className="hidden grid-cols-[minmax(0,1fr)_120px_88px] gap-3 px-2 py-2 text-xs font-medium leading-4 text-muted-foreground sm:grid">
            <div>Path</div>
            <div>Visibility</div>
            <div>Status</div>
          </div>
          {files.map((file) => (
            <div
              className="grid gap-2 px-2 py-2.5 text-sm sm:grid-cols-[minmax(0,1fr)_120px_88px] sm:items-center"
              key={file.path}
            >
              <div className="flex min-w-0 items-center gap-2">
                <File className="size-4 shrink-0 text-muted-foreground" />
                <span className="truncate font-mono">{file.path}</span>
              </div>
              <div>
                <VisibilityBadge visibility={file.visibility} />
              </div>
              <div>
                <Badge variant={file.tracked ? 'neutral' : 'warning'}>
                  {file.tracked ? 'Tracked' : 'Missing'}
                </Badge>
              </div>
            </div>
          ))}
        </div>
      )}
    </section>
  )
}

export function RepoDetailError({ error }: { error: unknown }) {
  return (
    <RouteErrorPage
      error={error}
      fallbackMessage="Unexpected repository error"
      title="Repository unavailable"
    />
  )
}
