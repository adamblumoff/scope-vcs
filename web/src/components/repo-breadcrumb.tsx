import type { RepoParams } from '@/api/types'
import { Link } from '@tanstack/react-router'

type RepoSection = 'history' | 'settings'

const SECTION_LABEL: Record<RepoSection, string> = {
  history: 'History',
  settings: 'Settings',
}

export function RepoBreadcrumb({
  params,
  section,
}: {
  params: RepoParams
  section?: RepoSection
}) {
  const repoId = `${params.owner}/${params.repo}`

  return (
    <nav
      aria-label="Breadcrumb"
      className="flex min-w-0 items-center gap-2 text-sm"
    >
      {section ? (
        <Link
          className="truncate font-mono text-muted-foreground transition-colors hover:text-foreground"
          params={params}
          to="/repos/$owner/$repo"
        >
          {repoId}
        </Link>
      ) : (
        <span className="truncate font-mono font-medium text-foreground">
          {repoId}
        </span>
      )}
      {section && (
        <>
          <span aria-hidden className="text-muted-foreground/50">
            /
          </span>
          <span className="shrink-0 font-medium text-foreground">
            {SECTION_LABEL[section]}
          </span>
        </>
      )}
    </nav>
  )
}
