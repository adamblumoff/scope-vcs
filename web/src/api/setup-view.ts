import type {
  RepoGitCredential,
  RepoGitCredentialView,
  RepoCloneCredential,
  RepoCloneCredentialView,
  RepoSetup,
  RepoSetupView,
} from './types'

export function setupView(api: string, setup: RepoSetup): RepoSetupView {
  return {
    ...setup,
    git_remote_url: gitRemoteUrl(api, setup.git_remote_path),
  }
}

export function repoGitCredentialView(
  api: string,
  credential: RepoGitCredential,
): RepoGitCredentialView {
  return {
    ...credential,
    git_remote_url: gitRemoteUrl(api, credential.git_remote_path),
  }
}

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
