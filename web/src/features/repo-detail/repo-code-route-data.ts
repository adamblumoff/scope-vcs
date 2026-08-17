import type { RepoContent, RepoFileContent } from '@/api/types'

export const DEFAULT_REPO_FILE_PATH = 'README.html'

export type RepoCodeRouteData = Awaited<
  ReturnType<typeof loadRepoCodeRouteData>
>

export async function loadRepoCodeRouteData({
  loadContent,
  loadFile,
  requestedPath,
}: {
  loadContent: () => Promise<RepoContent>
  loadFile: (path: string) => Promise<RepoFileContent | null>
  requestedPath?: string
}) {
  const content = loadContent()
  const selectedFile = await loadFile(requestedPath ?? DEFAULT_REPO_FILE_PATH)

  return {
    content,
    selectedFile,
    selectedPath: selectedFile?.path ?? null,
  }
}
