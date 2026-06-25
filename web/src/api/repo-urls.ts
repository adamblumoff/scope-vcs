import type { RepoCloneCredential, RepoCloneCredentialView } from './types'

export function repoCloneCredentialView(
  api: string,
  credential: RepoCloneCredential,
): RepoCloneCredentialView {
  return {
    ...credential,
    git_remote_url: gitRemoteUrl(api, credential.git_remote_path),
  }
}

export function gitRemoteUrl(api: string, path: string) {
  return `${stripTrailingSlash(api)}${path}`
}

function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}
