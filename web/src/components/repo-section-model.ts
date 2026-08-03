import type { RepositoryActor } from '@/api/types'

export type RepoSection = 'code' | 'history' | 'requests' | 'runs' | 'settings'

export const REPO_SECTIONS = [
  { key: 'code', label: 'Code', to: '/$owner/$repo' },
  { key: 'requests', label: 'Requests', to: '/$owner/$repo/requests' },
  { key: 'runs', label: 'Runs', to: '/$owner/$repo/runs' },
  { key: 'history', label: 'History', to: '/$owner/$repo/history' },
  { key: 'settings', label: 'Settings', to: '/$owner/$repo/settings' },
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
  if (matches('/$owner/$repo/settings')) return 'settings'
  if (matches('/$owner/$repo/history')) return 'history'
  if (matches('/$owner/$repo/runs')) return 'runs'
  if (matches('/$owner/$repo/requests')) return 'requests'
  return 'code'
}
