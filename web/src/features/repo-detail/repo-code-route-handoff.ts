import type { RepoParams } from '@/api/types'
import {
  DEFAULT_REPO_FILE_PATH,
  type RepoCodeRouteData,
} from './repo-code-route-data'

type RepoCodeLoadIdentity = RepoParams & { path?: string }

export function createRepoCodeRouteHandoff() {
  let pending: { data: RepoCodeRouteData; key: string } | null = null

  return {
    stage(input: RepoCodeLoadIdentity, data: RepoCodeRouteData) {
      pending = { data, key: repoCodeLoadKey(input) }
    },
    take(input: RepoCodeLoadIdentity) {
      const staged = pending
      pending = null
      return staged?.key === repoCodeLoadKey(input) ? staged.data : null
    },
  }
}

function repoCodeLoadKey(input: RepoCodeLoadIdentity) {
  return [
    input.owner,
    input.repo,
    input.path ?? DEFAULT_REPO_FILE_PATH,
  ].join('\0')
}
