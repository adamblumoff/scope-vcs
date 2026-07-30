import type { RepositoryActor } from '@/api/types'

export type RepoSection = 'code' | 'history' | 'requests' | 'runs' | 'settings'

export const REPO_SECTIONS = [
  { key: 'code', label: 'Code', to: '/repos/$owner/$repo' },
  { key: 'requests', label: 'Requests', to: '/repos/$owner/$repo/requests' },
  { key: 'runs', label: 'Runs', to: '/repos/$owner/$repo/runs' },
  { key: 'history', label: 'History', to: '/repos/$owner/$repo/history' },
  { key: 'settings', label: 'Settings', to: '/repos/$owner/$repo/settings' },
] as const

type RepoRoute = (typeof REPO_SECTIONS)[number]['to']

export function repoSectionsForActor(actor: RepositoryActor) {
  return actor === 'Public'
    ? REPO_SECTIONS.filter(({ key }) => key !== 'runs' && key !== 'settings')
    : REPO_SECTIONS
}

export function activeRepoSection(
  matches: (route: RepoRoute) => boolean,
): RepoSection {
  if (matches('/repos/$owner/$repo/settings')) return 'settings'
  if (matches('/repos/$owner/$repo/history')) return 'history'
  if (matches('/repos/$owner/$repo/runs')) return 'runs'
  if (matches('/repos/$owner/$repo/requests')) return 'requests'
  return 'code'
}
