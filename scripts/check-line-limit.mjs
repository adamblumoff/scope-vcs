import { readdir, readFile } from 'node:fs/promises'
import path from 'node:path'

const root = process.cwd()
const maxLines = 1000
const sourceExtensions = new Set([
  '.css',
  '.cjs',
  '.js',
  '.jsx',
  '.mjs',
  '.rs',
  '.ts',
  '.tsx',
])
const ignoredDirectories = new Set([
  '.agents',
  '.codex',
  '.codex-local',
  '.git',
  '.output',
  '.railway',
  '.scope',
  '.test-output',
  '.turbo',
  '.tmp',
  'dist',
  'node_modules',
  'target',
])
const ignoredFiles = new Set([
  'Cargo.lock',
  'package-lock.json',
  'pnpm-lock.yaml',
  'routeTree.gen.ts',
  'yarn.lock',
])

const oversized = []

for await (const file of sourceFiles(root)) {
  const contents = await readFile(file, 'utf8')
  const lines = countLines(contents)
  if (lines > maxLines) {
    oversized.push({ file: path.relative(root, file), lines })
  }
}

if (oversized.length > 0) {
  console.error(`Source files over ${maxLines} lines:`)
  for (const { file, lines } of oversized) {
    console.error(`  ${lines.toString().padStart(5)} ${file}`)
  }
  process.exitCode = 1
}

async function* sourceFiles(directory) {
  const entries = await readdir(directory, { withFileTypes: true })

  for (const entry of entries) {
    const fullPath = path.join(directory, entry.name)
    if (entry.isDirectory()) {
      if (!ignoredDirectories.has(entry.name)) {
        yield* sourceFiles(fullPath)
      }
      continue
    }

    if (!entry.isFile() || ignoredFiles.has(entry.name)) {
      continue
    }

    if (entry.name.endsWith('.gen.ts')) {
      continue
    }

    if (sourceExtensions.has(path.extname(entry.name))) {
      yield fullPath
    }
  }
}

function countLines(contents) {
  if (contents.length === 0) {
    return 0
  }

  const trailingNewline = contents.endsWith('\n') ? 1 : 0
  return contents.split('\n').length - trailingNewline
}
