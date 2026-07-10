import type { RepoContent, RepoFile, RepoFileContent, RepoParams } from '@/api/types'
import { LifecycleBadge } from '@/components/lifecycle-badge'
import { PageContent, PageHeader } from '@/components/page-header'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { RepoShell } from '@/components/repo-shell'
import { RouteErrorPage } from '@/components/route-error-page'
import { Badge } from '@/components/ui/badge'
import { RepoCloneDropdown } from './repo-clone-dropdown'
import { useRepoLayout } from './repo-layout-context'
import { RepositoryCodeView } from './repository-code-view'

export function RepoDetailPage({
  content,
  onSelectFile,
  params,
  selectedFile,
  selectedFileError,
  selectedPath,
}: {
  content: RepoContent
  onSelectFile: (file: RepoFile) => void
  params: RepoParams
  selectedFile: RepoFileContent | null
  selectedFileError: string | null
  selectedPath: string | null
}) {
  const { repo } = useRepoLayout()
  const files = content.files
  return (
    <RepoShell params={params}>
      <PageContent>
        <PageHeader
          actions={(
            <>
              {repo.lifecycle_state === 'Published' && (
                <RepoCloneDropdown
                  cloneRemoteUrl={content.clone_remote_url}
                  repo={repo}
                />
              )}
              <RepoPrimaryActionButton
                includeOpen={false}
                repo={repo}
                requireOwner
                variant="default"
              />
            </>
          )}
          badges={(
            <>
              <LifecycleBadge state={repo.lifecycle_state} />
              <Badge variant="neutral">{repo.access.actor}</Badge>
            </>
          )}
          title={repo.id}
          titleClassName="font-mono"
        />

        <RepositoryCodeView
          files={files}
          onSelectFile={onSelectFile}
          selectedFile={selectedFile}
          selectedFileError={selectedFileError}
          selectedPath={selectedPath}
        />
      </PageContent>
    </RepoShell>
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
