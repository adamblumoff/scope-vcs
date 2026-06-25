import {
  defaultGitCommandShell,
  gitCredentialApproveCommandForShell,
  gitCredentialFields,
  gitCredentialStoreCommands,
  gitRemoteUrlWithUsername,
  joinShellCommands,
  shellArg,
  type GitCommandShell,
} from '../git-command-shell'

export type RepoCloneCommandSource = {
  git_remote_url: string
}

const gitCredentialUsername = 'scope'

export function publicCloneCommand(
  source: RepoCloneCommandSource,
  shell: GitCommandShell = defaultGitCommandShell,
) {
  return `git clone ${shellArg(shell, source.git_remote_url)}`
}

export function credentialedCloneCommand(
  source: RepoCloneCommandSource,
  gitCloneTokenSecret: string,
  shell: GitCommandShell = defaultGitCommandShell,
) {
  const remoteUrl = gitCloneCredentialRemoteUrl(source.git_remote_url)
  return joinShellCommands(shell, [
    ...gitCredentialStoreCommands(shell, remoteUrl),
    gitCloneCredentialApproveCommand(shell, remoteUrl, gitCloneTokenSecret),
    `git clone -c ${shellArg(
      shell,
      'http.proactiveAuth=basic',
    )} ${shellArg(shell, remoteUrl)}`,
  ])
}

function gitCloneCredentialApproveCommand(
  shell: GitCommandShell,
  remoteUrl: string,
  gitCloneTokenSecret: string,
) {
  return gitCredentialApproveCommandForShell({
    fields: gitCredentialFields({
      password: gitCloneTokenSecret,
      remoteUrl,
      username: gitCredentialUsername,
    }),
    shell,
  })
}

function gitCloneCredentialRemoteUrl(remoteUrl: string) {
  return gitRemoteUrlWithUsername(remoteUrl, gitCredentialUsername)
}
