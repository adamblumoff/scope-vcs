export type RepoSetupCommandSource = {
  git_remote_url: string
  push_branch: string
  remote_name: string
}

export function setupCommand(
  setup: RepoSetupCommandSource,
  gitPushTokenSecret: string,
) {
  const remoteUrl = gitCredentialRemoteUrl(setup.git_remote_url)
  return `${gitCredentialApproveCommand(setup, gitPushTokenSecret)}; git remote remove ${setup.remote_name} 2>$null; git remote add ${setup.remote_name} ${remoteUrl}; git push ${setup.remote_name} HEAD:${setup.push_branch}`
}

export function gitCredentialApproveCommand(
  setup: Pick<RepoSetupCommandSource, 'git_remote_url'>,
  gitPushTokenSecret: string,
) {
  const remoteUrl = gitCredentialRemoteUrl(setup.git_remote_url)
  const credential = gitCredentialFields(remoteUrl, gitPushTokenSecret)
  return `git config credential.useHttpPath true; "${credential}" | git credential approve`
}

function gitCredentialRemoteUrl(remoteUrl: string) {
  try {
    const url = new URL(remoteUrl)
    url.username = 'scope'
    url.password = ''
    return url.toString()
  } catch {
    return remoteUrl
  }
}

function gitCredentialFields(remoteUrl: string, gitPushTokenSecret: string) {
  const url = new URL(remoteUrl)
  const fields = [
    'protocol=https',
    `host=${powerShellDoubleQuoted(url.host)}`,
    `path=${powerShellDoubleQuoted(gitCredentialPath(url))}`,
    'username=scope',
    `password=${powerShellDoubleQuoted(gitPushTokenSecret)}`,
    '',
    '',
  ]
  return fields.join('`n')
}

function gitCredentialPath(url: URL) {
  return url.pathname.replace(/^\/+/, '')
}

function powerShellDoubleQuoted(value: string) {
  return value.replaceAll('`', '``').replaceAll('$', '`$').replaceAll('"', '`"')
}
