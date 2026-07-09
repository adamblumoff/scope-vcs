import type { RepoParams } from '@/api/types'
import { Link } from '@tanstack/react-router'

export function RepoBreadcrumb({
  params,
}: {
  params: RepoParams
}) {
  const repoId = `${params.owner}/${params.repo}`

  return (
    <nav
      aria-label="Breadcrumb"
      className="flex min-w-0 items-center gap-2 text-sm"
    >
      <Link
        className="truncate font-mono font-medium text-foreground hover:text-brand"
        params={params}
        to="/repos/$owner/$repo"
      >
        {repoId}
      </Link>
    </nav>
  )
}
