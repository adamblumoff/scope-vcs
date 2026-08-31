import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const sourceRoot = path.join(webRoot, 'src')
const sourceFiles = await collectSourceFiles(sourceRoot)
const violations = []

for (const file of sourceFiles) {
  const relativePath = path.relative(webRoot, file).replaceAll(path.sep, '/')
  const source = await readFile(file, 'utf8')
  const importsPagent = /from\s+['"]pagent['"]/.test(source)
  if (importsPagent && !relativePath.startsWith('src/server/pagent-')) {
    violations.push(`${relativePath} imports the optional Pagent package`)
  }
  if (
    /pagent/i.test(source) &&
    (
      relativePath.startsWith('src/api/') ||
      relativePath === 'src/features/repo-detail/repo-event-stream.ts' ||
      relativePath === 'src/features/repo-detail/repo-live-refresh.ts'
    )
  ) {
    violations.push(`${relativePath} names Pagent inside Scope-owned behavior`)
  }
}

if (violations.length > 0) {
  console.error(violations.join('\n'))
  process.exit(1)
}

async function collectSourceFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name)
    if (entry.isDirectory()) files.push(...await collectSourceFiles(entryPath))
    else if (/\.[cm]?[jt]sx?$/.test(entry.name)) files.push(entryPath)
  }
  return files
}
