import type { RepoDetail, RepoFile, RepoParams } from '@/api/types'
import { AppHeader } from '@/components/app-header'
import { FileSystemTreePanel } from '@/components/file-system-tree'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { RepoBreadcrumb } from '@/components/repo-breadcrumb'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { RouteErrorPage } from '@/components/route-error-page'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import { GitPullRequest, Settings } from 'lucide-react'
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
              <Button asChild size="sm" variant="secondary">
                <Link
                  params={{ owner: repo.owner_handle, repo: repo.name }}
                  to="/repos/$owner/$repo/requests"
                >
                  <GitPullRequest className="size-3.5" />
                  <span>Requests</span>
                </Link>
              </Button>
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
              <Badge variant={repo.open_request_count > 0 ? 'info' : 'neutral'}>
                {repo.open_request_count} requests
              </Badge>
            </>
          )}
          title={repo.id}
          titleClassName="font-mono"
        />

        <FileSystemTreePanel
          description={
            publicOnlyView
              ? 'Public files available from this scoped repo.'
              : 'Files in the latest scoped repo view.'
          }
          emptyDescription={
            publicOnlyView
              ? 'Run scope push with public files in the repo config to show files here.'
              : 'Run scope push from the CLI to add files to this repo.'
          }
          emptyTitle={publicOnlyView ? 'No public files' : 'No files'}
          files={files}
          getFileMeta={repoFileStatus}
          metaColumnLabel="Status"
        />
      </PageContent>
    </main>
  )
}

function repoFileStatus(file: RepoFile) {
  return (
    <Badge variant={file.tracked ? 'neutral' : 'warning'}>
      {file.tracked ? 'Tracked' : 'Missing'}
    </Badge>
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
