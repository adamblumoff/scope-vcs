import type { RepoContent, RepoFileContent, RepoParams } from '@/api/types'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { RepoShell } from '@/components/repo-shell'
import { RouteErrorPage } from '@/components/route-error-page'
import { WorkbenchHeader } from '@/components/workbench-header'
import { RepoCloneDropdown } from './repo-clone-dropdown'
import { useRepoLayout } from './repo-layout-context'
import { RepositoryCodeView } from './repository-code-view'

export function RepoDetailPage({
  content,
  onSelectFilePath,
  params,
  selectedFile,
  selectedFileError,
  selectedPath,
}: {
  content: RepoContent
  onSelectFilePath: (path: string | null) => void
  params: RepoParams
  selectedFile: RepoFileContent | null
  selectedFileError: string | null
  selectedPath: string | null
}) {
  const { repo } = useRepoLayout()
  const files = content.files
  return (
    <RepoShell params={params}>
      <WorkbenchHeader
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
        count={`${files.length} ${files.length === 1 ? 'file' : 'files'}`}
        description="Browse the latest scoped repository view."
        eyebrow="Browse"
        title="Repository"
      />
      <RepositoryCodeView
        files={files}
        onSelectFilePath={onSelectFilePath}
        params={params}
        selectedFile={selectedFile}
        selectedFileError={selectedFileError}
        selectedPath={selectedPath}
      />
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
