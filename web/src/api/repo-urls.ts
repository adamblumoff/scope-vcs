export function gitRemoteUrl(api: string, path: string) {
  return `${stripTrailingSlash(api)}${path}`
}

function stripTrailingSlash(value: string) {
  return value.replace(/\/+$/, '')
}
