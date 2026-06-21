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
  const credential = gitCredentialFields(remoteUrl, gitPushTokenSecret)
  return `"${credential}" | git credential approve; git remote remove ${setup.remote_name} 2>$null; git remote add ${setup.remote_name} ${remoteUrl}; git push ${setup.remote_name} HEAD:${setup.push_branch}`
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
  return [
    'protocol=https',
    `host=${powerShellDoubleQuoted(url.host)}`,
    'username=scope',
    `password=${powerShellDoubleQuoted(gitPushTokenSecret)}`,
    '',
    '',
  ].join('`n')
}

function powerShellDoubleQuoted(value: string) {
  return value.replaceAll('`', '``').replaceAll('$', '`$').replaceAll('"', '`"')
}
