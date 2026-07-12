export function displayRouteFilePath(path: string) {
  return path.replace(/^\/+/, '')
}

export function parseRouteFileSearch(value: unknown) {
  if (typeof value !== 'string') return undefined
  const path = displayRouteFilePath(value)
  return path && !path.split('/').some((part) => part === '.' || part === '..')
    ? path
    : undefined
}

export function selectedRouteFilePath(
  files: ReadonlyArray<{ path: string }>,
  selected?: string,
) {
  if (!selected) return null
  return (
    files.find((file) => displayRouteFilePath(file.path) === selected)?.path ??
    null
  )
}

export function defaultReadmePath(files: ReadonlyArray<{ path: string }>) {
  const readme = files.find((file) =>
    /^\/?readme(?:\.[^/]+)?$/i.test(file.path),
  )
  return readme ? displayRouteFilePath(readme.path) : undefined
}

export function routeErrorMessage(error: unknown, fallback: string) {
  return error instanceof Error && error.message.trim() ? error.message : fallback
}
