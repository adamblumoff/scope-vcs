export type RepoSetupCommandSource = {
  git_remote_url: string
  push_branch: string
  remote_name: string
}

export function setupCommands(setup: RepoSetupCommandSource) {
  return [
    `git remote add ${setup.remote_name} ${gitCredentialRemoteUrl(setup.git_remote_url)}`,
    `git push -u ${setup.remote_name} HEAD:${setup.push_branch}`,
  ]
}

export function dualRemotePushCommands(setup: RepoSetupCommandSource) {
  return [
    'git remote get-url origin',
    'git remote set-url --add --push origin <github-remote-url>',
    `git remote set-url --add --push origin ${gitCredentialRemoteUrl(setup.git_remote_url)}`,
    `git push origin HEAD:${setup.push_branch}`,
  ]
}

export function gitCredentialHost(remoteUrl: string) {
  try {
    return new URL(remoteUrl).host
  } catch {
    return remoteUrl
  }
}

function gitCredentialRemoteUrl(remoteUrl: string) {
  try {
    const url = new URL(remoteUrl)
    url.username = 'scope'
    return url.toString()
  } catch {
    return remoteUrl
  }
}
