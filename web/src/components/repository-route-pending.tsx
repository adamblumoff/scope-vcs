import { ApplicationTopbar } from '@/components/application-topbar'
import { AppShell } from '@/components/app-shell'
import { HistoryPagePending } from '@/features/history/history-page-pending'
import { RepoSettingsPending } from '@/features/repo-detail/repo-settings-pending'
import { RepositoryCodePending } from '@/features/repo-detail/repository-code-pending'
import { RequestDetailPagePending } from '@/features/requests/request-page-pending'
import { RequestsPagePending } from '@/features/requests/requests-page-pending'
import { RunDetailPagePending } from '@/features/runs/run-detail-pending'
import { RunsPagePending } from '@/features/runs/runs-page-pending'
import { useLocation, useParams } from '@tanstack/react-router'

export function RepositoryRoutePending() {
  const repository = useParams({ from: '/$owner/$repo' })
  const pathname = useLocation({ select: (location) => location.pathname })

  return (
    <AppShell
      header={() => <ApplicationTopbar repository={repository} />}
    >
      <RepositoryBodyPending pathname={pathname} />
    </AppShell>
  )
}

function RepositoryBodyPending({ pathname }: { pathname: string }) {
  const routePath = pathname.replace(/\/+$/, '')
  if (routePath.includes('/requests/')) return <RequestDetailPagePending />
  if (routePath.endsWith('/requests')) return <RequestsPagePending />
  if (routePath.endsWith('/history')) return <HistoryPagePending />
  if (/\/runs\/[^/]+$/.test(routePath) && !routePath.includes('/workflows/')) {
    return <RunDetailPagePending />
  }
  if (routePath.includes('/runs')) return <RunsPagePending />
  if (routePath.endsWith('/settings')) return <RepoSettingsPending />
  return <RepositoryCodePending />
}
