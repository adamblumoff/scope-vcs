import type { RepoSetup, RepoSetupView } from './types'

export function setupView(api: string, setup: RepoSetup): RepoSetupView {
  const gitRemoteUrl = `${stripTrailingSlash(api)}${setup.git_remote_path}`

  return {
    ...setup,
    git_remote_url: gitRemoteUrl,
  }
}

function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}
