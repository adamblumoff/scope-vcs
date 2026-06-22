export function setupPushSecretKey(repoId: string) {
  return `scope:git-push-token:${repoId}`
}
