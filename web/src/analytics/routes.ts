export type AnalyticsRoute = {
  name: string
  path: string
}

const routeAliases: Readonly<Record<string, AnalyticsRoute>> = {
  '/': { name: 'home', path: '/' },
  '/account': { name: 'account', path: '/account' },
  '/cli-login': { name: 'cli_login', path: '/cli-login' },
  '/invites/$token': { name: 'invite', path: '/invite' },
  '/sign-in/$': { name: 'sign_in', path: '/sign-in' },
  '/sign-up/$': { name: 'sign_up', path: '/sign-up' },
  '/$owner': { name: 'owner', path: '/owner' },
  '/$owner/': { name: 'owner', path: '/owner' },
  '/$owner/$repo': { name: 'repository', path: '/repository' },
  '/$owner/$repo/_code': {
    name: 'repository_code',
    path: '/repository/code',
  },
  '/$owner/$repo/_code/': {
    name: 'repository_code',
    path: '/repository/code',
  },
  '/$owner/$repo/history': {
    name: 'repository_history',
    path: '/repository/history',
  },
  '/$owner/$repo/requests': {
    name: 'repository_requests',
    path: '/repository/requests',
  },
  '/$owner/$repo/requests/': {
    name: 'repository_requests',
    path: '/repository/requests',
  },
  '/$owner/$repo/requests/$requestId': {
    name: 'request',
    path: '/repository/request',
  },
  '/$owner/$repo/requests/$requestId/': {
    name: 'request',
    path: '/repository/request',
  },
  '/$owner/$repo/requests/$requestId/changes': {
    name: 'request_changes',
    path: '/repository/request/changes',
  },
  '/$owner/$repo/runs': {
    name: 'repository_runs',
    path: '/repository/runs',
  },
  '/$owner/$repo/runs/': {
    name: 'repository_runs',
    path: '/repository/runs',
  },
  '/$owner/$repo/runs/$runId': {
    name: 'repository_run',
    path: '/repository/run',
  },
  '/$owner/$repo/runs/workflows/$workflow': {
    name: 'repository_workflow',
    path: '/repository/workflow',
  },
  '/$owner/$repo/settings': {
    name: 'repository_settings',
    path: '/repository/settings',
  },
}

const routesByName = new Map(
  Object.values(routeAliases).map((route) => [route.name, route]),
)

export function analyticsRouteForId(routeId: string | undefined) {
  return routeId ? routeAliases[routeId] ?? null : null
}

export function analyticsRouteForName(name: string) {
  return routesByName.get(name) ?? null
}
