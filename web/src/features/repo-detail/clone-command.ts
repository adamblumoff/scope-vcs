import {
  defaultGitCommandShell,
  gitCredentialApproveCommandForShell,
  gitCredentialFields,
  gitCredentialUseHttpPathConfig,
  gitRemoteUrlWithUsername,
  joinShellCommands,
  shellArg,
  type GitCommandShell,
} from '../git-command-shell'

export type RepoCloneCommandSource = {
  git_remote_url: string
}

const cloneCredentialUsername = 'scope-clone'

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
  const useHttpPathConfig = gitCredentialUseHttpPathConfig(remoteUrl)
  const useHttpPathConfigArg = `${useHttpPathConfig}=true`
  return joinShellCommands(shell, [
    gitCloneCredentialApproveCommand(
      shell,
      remoteUrl,
      gitCloneTokenSecret,
      useHttpPathConfigArg,
    ),
    `git clone -c ${shellArg(shell, useHttpPathConfigArg)} -c ${shellArg(
      shell,
      'http.proactiveAuth=basic',
    )} ${shellArg(shell, remoteUrl)}`,
  ])
}

function gitCloneCredentialApproveCommand(
  shell: GitCommandShell,
  remoteUrl: string,
  gitCloneTokenSecret: string,
  useHttpPathConfigArg: string,
) {
  return gitCredentialApproveCommandForShell({
    fields: gitCredentialFields({
      password: gitCloneTokenSecret,
      remoteUrl,
      username: cloneCredentialUsername,
    }),
    gitConfigArgs: ['-c', useHttpPathConfigArg],
    shell,
  })
}

function gitCloneCredentialRemoteUrl(remoteUrl: string) {
  return gitRemoteUrlWithUsername(remoteUrl, cloneCredentialUsername)
}
