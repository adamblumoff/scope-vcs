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
