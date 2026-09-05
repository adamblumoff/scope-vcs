import type {
  RepoContent,
  RepoFileContent,
  RepoParams,
  RepoSummary,
} from '@/api/types'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { RepoCloneDropdown } from './repo-clone-dropdown'
import { RepositoryCodeView } from './repository-code-view'

export function RepoDetailPage({
  content,
  contentError,
  contentLoading,
  contentRetry,
  onSelectFilePath,
  params,
  repo,
  selectedFile,
  selectedFileError,
  selectedFileIdentity,
  selectedFileLoading,
  selectedFileRetry,
  selectedPath,
}: {
  content: RepoContent | null
  contentError: string | null
  contentLoading: boolean
  contentRetry: () => void
  onSelectFilePath: (path: string) => void
  params: RepoParams
  repo: RepoSummary
  selectedFile: RepoFileContent | null
  selectedFileError: string | null
  selectedFileIdentity: string | null
  selectedFileLoading: boolean
  selectedFileRetry: () => void
  selectedPath: string | null
}) {
  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <>
            {content && repo.lifecycle_state === 'Ready' && (
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
        summary={content
          ? `${content.files.length} ${content.files.length === 1 ? 'file' : 'files'}`
          : contentLoading ? undefined : 'Files unavailable'}
        title="Code"
      />
      <RepositoryCodeView
        content={content}
        contentError={contentError}
        contentRetry={contentRetry}
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
