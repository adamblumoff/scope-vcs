export type GitCommandShell = 'posix' | 'powershell'

export const defaultGitCommandShell: GitCommandShell = 'posix'

export const gitCommandShellOptions = [
  { label: 'Bash/Zsh', value: 'posix' },
  { label: 'PowerShell', value: 'powershell' },
] as const

export function isGitCommandShell(value: string): value is GitCommandShell {
  return value === 'posix' || value === 'powershell'
}

export function gitCredentialFields({
  password,
  remoteUrl,
  username,
}: {
  password: string
  remoteUrl: string
  username: string
}) {
  const url = new URL(remoteUrl)
  return [
    `protocol=${url.protocol.replace(/:$/, '')}`,
    `host=${url.host}`,
    `path=${gitCredentialPath(url)}`,
    `username=${username}`,
    `password=${password}`,
    '',
  ]
}

export function gitCredentialUseHttpPathConfig(remoteUrl: string) {
  const url = new URL(remoteUrl)
  return `credential.${url.protocol}//${url.host}.useHttpPath`
}

export function gitCredentialStoreSetupCommands(
  shell: GitCommandShell,
  remoteUrl: string,
) {
  return [
    ...gitCredentialStoreDirectoryCommands(shell),
    gitCredentialStoreHelperConfigCommand(shell, remoteUrl),
    gitConfigSetCommand(
      shell,
      gitCredentialUseHttpPathConfig(remoteUrl),
      'true',
      { global: true },
    ),
  ]
}

export function gitRemoteUrlWithUsername(remoteUrl: string, username: string) {
  try {
    const url = new URL(remoteUrl)
    url.username = username
    url.password = ''
    return url.toString()
  } catch {
    return remoteUrl
  }
}

export function gitConfigSetCommand(
  shell: GitCommandShell,
  key: string,
  value: string,
  options: { global?: boolean } = {},
) {
  const scopeArg = options.global ? ' --global' : ''
  return `git config${scopeArg} --replace-all ${shellArg(shell, key)} ${shellArg(
    shell,
    value,
  )}`
}

export function gitConfigRemoveSectionCommand(
  shell: GitCommandShell,
  section: string,
) {
  const command = `git config --remove-section ${shellArg(shell, section)}`
  return shell === 'posix'
    ? `(${command} >/dev/null 2>&1 || true)`
    : `${command} 2>$null`
}

export function gitCredentialApproveCommandForShell({
  fields,
  gitConfigArgs = [],
  shell,
}: {
  fields: string[]
  gitConfigArgs?: string[]
  shell: GitCommandShell
}) {
  const gitArgs = gitConfigArgs
    .map((arg) => (arg === '-c' ? arg : shellArg(shell, arg)))
    .join(' ')
  const gitCommand = gitArgs
    ? `git ${gitArgs} credential approve`
    : 'git credential approve'
  return `${credentialInputCommand(shell, fields)} | ${gitCommand}`
}

export function joinShellCommands(
  shell: GitCommandShell,
  commands: string[],
) {
  return commands.join(shell === 'posix' ? ' && ' : '; ')
}

export function shellArg(shell: GitCommandShell, value: string) {
  switch (shell) {
    case 'posix':
      return posixSingleQuoted(value)
    case 'powershell':
      return powerShellSingleQuoted(value)
  }
}

function credentialInputCommand(shell: GitCommandShell, fields: string[]) {
  switch (shell) {
    case 'posix':
      return `printf '%s\\n' ${fields
        .map((field) => shellArg(shell, field))
        .join(' ')}`
    case 'powershell':
      return `@(${fields
        .map((field) => shellArg(shell, field))
        .join(', ')})`
  }
}

function gitCredentialPath(url: URL) {
  return url.pathname.replace(/^\/+/, '')
}

function gitCredentialStoreHelperConfig(remoteUrl: string) {
  const url = new URL(remoteUrl)
  return `credential.${url.protocol}//${url.host}.helper`
}

function gitCredentialStoreDirectoryCommands(shell: GitCommandShell) {
  switch (shell) {
    case 'posix':
      return ['mkdir -p ~/.config/scope', 'chmod 700 ~/.config/scope']
    case 'powershell':
      return [
        "$scopeCredentialDir = ($env:USERPROFILE -replace '\\\\', '/') + '/.config/scope'",
        'New-Item -ItemType Directory -Force $scopeCredentialDir | Out-Null',
        '$scopeCredentialFile = "$scopeCredentialDir/git-credentials"',
      ]
  }
}

function gitCredentialStoreHelperConfigCommand(
  shell: GitCommandShell,
  remoteUrl: string,
) {
  const key = gitCredentialStoreHelperConfig(remoteUrl)
  switch (shell) {
    case 'posix':
      return gitConfigSetCommand(
        shell,
        key,
        'store --file ~/.config/scope/git-credentials',
        { global: true },
      )
    case 'powershell':
      return `git config --global --replace-all ${shellArg(
        shell,
        key,
      )} "store --file \`"$scopeCredentialFile\`""`
  }
}

function posixSingleQuoted(value: string) {
  return `'${value.replaceAll("'", "'\\''")}'`
}

function powerShellSingleQuoted(value: string) {
  return `'${value.replaceAll("'", "''")}'`
}
