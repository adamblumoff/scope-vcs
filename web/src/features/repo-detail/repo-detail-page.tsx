import type {
  RepoContent,
  RepoFileContent,
  RepoParams,
} from '@/api/types'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { RepoCloneDropdown } from './repo-clone-dropdown'
import { useRepoLayout } from './repo-layout-context'
import { RepositoryCodeView } from './repository-code-view'

export function RepoDetailPage({
  content,
  onSelectFilePath,
  params,
  selectedFile,
  selectedFileError,
  selectedFileIdentity,
  selectedFileLoading,
  selectedFileRetry,
  selectedPath,
}: {
  content: RepoContent
  onSelectFilePath: (path: string | null) => void
  params: RepoParams
  selectedFile: RepoFileContent | null
  selectedFileError: string | null
  selectedFileIdentity: string | null
  selectedFileLoading: boolean
  selectedFileRetry: () => void
  selectedPath: string | null
}) {
  const { repo } = useRepoLayout()
  const files = content.files
  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <>
            {repo.lifecycle_state === 'Ready' && (
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
        summary={`${files.length} ${files.length === 1 ? 'file' : 'files'}`}
        title="Code"
      />
      <RepositoryCodeView
        files={files}
        onSelectFilePath={onSelectFilePath}
        params={params}
        selectedFile={selectedFile}
        selectedFileError={selectedFileError}
        selectedFileIdentity={selectedFileIdentity}
        selectedFileLoading={selectedFileLoading}
        selectedFileRetry={selectedFileRetry}
        selectedPath={selectedPath}
      />
    </WorkbenchPane>
  )
}
