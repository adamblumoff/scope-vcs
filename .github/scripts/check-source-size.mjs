import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { promisify } from 'node:util'
import { fileURLToPath, pathToFileURL } from 'node:url'

export const reviewLines = 700
export const maxLines = 1000

const execFileAsync = promisify(execFile)
const sourceExtensions = new Set(['.cjs', '.css', '.js', '.jsx', '.mjs', '.rs', '.sh', '.ts', '.tsx'])
const ignoredFiles = new Set([
  'Cargo.lock',
  'package-lock.json',
  'pnpm-lock.yaml',
  'routeTree.gen.ts',
  'yarn.lock',
])

export function countLines(contents) {
  if (contents.length === 0) return 0
  return contents.split('\n').length - (contents.endsWith('\n') ? 1 : 0)
}

export function isSourceFile(file) {
  const normalized = file.replaceAll('\\', '/')
  const basename = path.posix.basename(normalized)
  return !ignoredFiles.has(basename)
    && !basename.endsWith('.gen.ts')
    && !basename.endsWith('.generated.ts')
    && (sourceExtensions.has(path.posix.extname(basename))
      || (normalized.startsWith('dev/') && path.posix.extname(basename) === ''))
}

export function sourceCategory(file) {
  const normalized = file.replaceAll('\\', '/')
  const basename = path.posix.basename(normalized)
  const segments = normalized.split('/')
  const supportSegment = segments.some((segment) => [
    'bench',
    'dev',
    'smoke',
    'tests',
    'workflow_tests',
  ].includes(segment))
  const supportName = basename === 'tests.rs'
    || basename === 'test_support.rs'
    || basename.includes('.spec.')
    || basename.includes('.test.')
    || basename.endsWith('_test.rs')
    || basename.endsWith('_tests.rs')
    || (normalized.startsWith('.github/scripts/') && basename.startsWith('test-'))
  return supportSegment || supportName ? 'test/support' : 'production'
}

export function evaluateSourceSizes(sources, auditEntries) {
  const reviewed = sources.filter(({ lines }) => lines >= reviewLines)
  const production = reviewed.filter(({ category }) => category === 'production')
  const support = reviewed.filter(({ category }) => category === 'test/support')
  const errors = []
  const auditByPath = new Map()

  for (const entry of auditEntries) {
    if (!entry || typeof entry.path !== 'string' || typeof entry.owner !== 'string'
      || typeof entry.reason !== 'string' || !entry.path || !entry.owner.trim() || !entry.reason.trim()) {
      errors.push('source-size audit entries require non-empty path, owner, and reason fields')
      continue
    }
    if (auditByPath.has(entry.path)) errors.push(`duplicate source-size audit entry: ${entry.path}`)
    auditByPath.set(entry.path, entry)
  }

  const sourceByPath = new Map(sources.map((source) => [source.path, source]))
  for (const source of production) {
    if (!auditByPath.has(source.path)) {
      errors.push(`${source.path}: production source at ${source.lines} lines needs an ownership audit`)
    }
  }
  for (const entry of auditByPath.values()) {
    const source = sourceByPath.get(entry.path)
    if (!source) errors.push(`${entry.path}: stale source-size audit points to a missing or excluded file`)
    else if (source.category !== 'production') errors.push(`${entry.path}: source-size audit may cover only production source`)
    else if (source.lines < reviewLines) errors.push(`${entry.path}: stale source-size audit remains below ${reviewLines} lines`)
  }
  for (const source of sources.filter(({ lines }) => lines > maxLines)) {
    errors.push(`${source.path}: ${source.lines} lines exceeds the ${maxLines}-line cap`)
  }

  return { errors, production, support }
}

async function trackedAndUntrackedSources(root) {
  const { stdout } = await execFileAsync(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard', '-z'],
    { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 },
  )
  const files = stdout.split('\0').filter(Boolean).filter(isSourceFile)
  const sources = await Promise.all(files.map(async (file) => {
    try {
      return {
        category: sourceCategory(file),
        lines: countLines(await readFile(path.join(root, file), 'utf8')),
        path: file,
      }
    } catch (error) {
      if (error.code === 'ENOENT') return null
      throw error
    }
  }))
  return sources.filter(Boolean)
}

function printReport(label, sources) {
  if (sources.length === 0) return
  console.log(`${label} (${reviewLines}+ lines):`)
  for (const source of [...sources].sort((left, right) => right.lines - left.lines || left.path.localeCompare(right.path))) {
    console.log(`  ${String(source.lines).padStart(5)} ${source.path}`)
  }
}

async function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
  const auditPath = path.join(root, '.github', 'source-size-audit.json')
  const audit = JSON.parse(await readFile(auditPath, 'utf8'))
  if (audit.version !== 1 || !Array.isArray(audit.production)) {
    throw new Error(`${path.relative(root, auditPath)} must contain version 1 and a production array`)
  }
  const result = evaluateSourceSizes(await trackedAndUntrackedSources(root), audit.production)
  printReport('Production sources requiring an ownership review', result.production)
  printReport('Test and support sources measured separately', result.support)
  if (result.errors.length > 0) {
    console.error(`Source-size guardrail failed:\n- ${result.errors.join('\n- ')}`)
    process.exitCode = 1
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
