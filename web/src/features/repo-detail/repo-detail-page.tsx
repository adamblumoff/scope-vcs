import type {
  RepoContent,
  RepoFileContent,
  RepoParams,
} from '@/api/types'
import { RepoPrimaryActionButton } from '@/components/repo-primary-action'
import { WorkbenchBar, WorkbenchPane } from '@/components/page-header'
import { Await } from '@tanstack/react-router'
import { RepoCloneDropdown } from './repo-clone-dropdown'
import { useRepoLayout } from './repo-layout-context'
import { RepositoryCodeView } from './repository-code-view'

export function RepoDetailPage({
  content,
  onSelectFilePath,
  params,
  selectedFile,
  selectedFileIdentity,
  selectedPath,
}: {
  content: Promise<RepoContent>
  onSelectFilePath: (path: string) => Promise<boolean>
  params: RepoParams
  selectedFile: RepoFileContent | null
  selectedFileIdentity: string | null
  selectedPath: string | null
}) {
  const { repo } = useRepoLayout()
  return (
    <WorkbenchPane>
      <WorkbenchBar
        actions={(
          <>
            <Await promise={content} fallback={null}>
              {(resolved) => repo.lifecycle_state === 'Ready' && (
                <RepoCloneDropdown
                  cloneRemoteUrl={resolved.clone_remote_url}
                  repo={repo}
                />
              )}
            </Await>
            <RepoPrimaryActionButton
              includeOpen={false}
              repo={repo}
              requireOwner
              variant="default"
            />
          </>
        )}
        summary={(
          <Await promise={content} fallback="Loading files…">
            {(resolved) => `${resolved.files.length} ${resolved.files.length === 1 ? 'file' : 'files'}`}
          </Await>
        )}
        title="Code"
      />
      <RepositoryCodeView
        content={content}
        onSelectFilePath={onSelectFilePath}
        params={params}
        selectedFile={selectedFile}
        selectedFileIdentity={selectedFileIdentity}
        selectedPath={selectedPath}
      />
    </WorkbenchPane>
  )
}
