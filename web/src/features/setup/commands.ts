import {
  defaultGitCommandShell,
  gitConfigRemoveSectionCommand,
  gitConfigSetCommand,
  gitCredentialApproveCommandForShell,
  gitCredentialFields,
  gitCredentialStoreSetupCommands,
  gitRemoteUrlWithUsername,
  joinShellCommands,
  shellArg,
  type GitCommandShell,
} from '../git-command-shell'

export type RepoSetupCommandSource = {
  git_remote_url: string
  push_branch: string
  remote_name: string
}

export function setupCommand(
  setup: RepoSetupCommandSource,
  gitPushTokenSecret: string,
  shell: GitCommandShell = defaultGitCommandShell,
) {
  const remoteUrl = gitCredentialRemoteUrl(setup.git_remote_url)
  return joinShellCommands(shell, [
    gitCredentialApproveCommand(setup, gitPushTokenSecret, shell),
    gitConfigRemoveSectionCommand(shell, `remote.${setup.remote_name}`),
    gitConfigSetCommand(shell, `remote.${setup.remote_name}.url`, remoteUrl),
    gitConfigSetCommand(
      shell,
      `remote.${setup.remote_name}.pushurl`,
      remoteUrl,
    ),
    gitConfigSetCommand(
      shell,
      `remote.${setup.remote_name}.fetch`,
      `+refs/heads/*:refs/remotes/${setup.remote_name}/*`,
    ),
    `git push ${shellArg(shell, setup.remote_name)} ${shellArg(
      shell,
      `HEAD:${setup.push_branch}`,
    )}`,
  ])
}

export function gitCredentialApproveCommand(
  setup: Pick<RepoSetupCommandSource, 'git_remote_url'>,
  gitPushTokenSecret: string,
  shell: GitCommandShell = defaultGitCommandShell,
) {
  const remoteUrl = gitCredentialRemoteUrl(setup.git_remote_url)
  const credential = gitCredentialFields({
    password: gitPushTokenSecret,
    remoteUrl,
    username: 'scope',
  })
  return joinShellCommands(shell, [
    ...gitCredentialStoreSetupCommands(shell, remoteUrl),
    gitCredentialApproveCommandForShell({ fields: credential, shell }),
  ])
}

export function gitCredentialRemoteUrl(remoteUrl: string) {
  return gitRemoteUrlWithUsername(remoteUrl, 'scope')
}
