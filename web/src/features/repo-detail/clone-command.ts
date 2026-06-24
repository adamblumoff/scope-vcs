export type RepoCloneCommandSource = {
  git_remote_url: string
}

const cloneCredentialUsername = 'scope-clone'

export function publicCloneCommand(source: RepoCloneCommandSource) {
  return `git clone ${source.git_remote_url}`
}

export function credentialedCloneCommand(
  source: RepoCloneCommandSource,
  gitCloneTokenSecret: string,
) {
  const remoteUrl = gitCloneCredentialRemoteUrl(source.git_remote_url)
  const useHttpPathConfig = gitCredentialUseHttpPathConfig(remoteUrl)
  return `${gitCloneCredentialApproveCommand(
    remoteUrl,
    gitCloneTokenSecret,
    useHttpPathConfig,
  )}; git clone -c "${useHttpPathConfig}=true" -c "http.proactiveAuth=basic" ${remoteUrl}`
}

function gitCloneCredentialApproveCommand(
  remoteUrl: string,
  gitCloneTokenSecret: string,
  useHttpPathConfig: string,
) {
  return `"${gitCloneCredentialFields(
    remoteUrl,
    gitCloneTokenSecret,
  )}" | git -c "${useHttpPathConfig}=true" credential approve`
}

function gitCloneCredentialFields(
  remoteUrl: string,
  gitCloneTokenSecret: string,
) {
  const url = new URL(remoteUrl)
  const fields = [
    `protocol=${powerShellDoubleQuoted(url.protocol.replace(/:$/, ''))}`,
    `host=${powerShellDoubleQuoted(url.host)}`,
    `path=${powerShellDoubleQuoted(gitCredentialPath(url))}`,
    `username=${cloneCredentialUsername}`,
    `password=${powerShellDoubleQuoted(gitCloneTokenSecret)}`,
    '',
    '',
  ]
  return fields.join('`n')
}

function gitCloneCredentialRemoteUrl(remoteUrl: string) {
  try {
    const url = new URL(remoteUrl)
    url.username = cloneCredentialUsername
    url.password = ''
    return url.toString()
  } catch {
    return remoteUrl
  }
}

function gitCredentialUseHttpPathConfig(remoteUrl: string) {
  const url = new URL(remoteUrl)
  const credentialUrl = `${url.protocol}//${url.host}`
  return `credential.${powerShellDoubleQuoted(credentialUrl)}.useHttpPath`
}

function gitCredentialPath(url: URL) {
  return url.pathname.replace(/^\/+/, '')
}

function powerShellDoubleQuoted(value: string) {
  return value.replaceAll('`', '``').replaceAll('$', '`$').replaceAll('"', '`"')
}
