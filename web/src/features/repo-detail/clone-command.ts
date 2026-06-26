export function permissionedCloneCommand(owner: string, repo: string) {
  return `scope clone ${owner}/${repo}`
}

export function publicCloneCommand(remoteUrl: string) {
  return `git clone ${remoteUrl}`
}
