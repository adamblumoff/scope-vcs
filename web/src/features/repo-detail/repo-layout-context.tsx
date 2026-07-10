import type { RepoLiveState } from '@/api/types'
import { createContext, type ReactNode, use } from 'react'

const RepoLayoutContext = createContext<RepoLiveState | null>(null)

export function RepoLayoutProvider({
  children,
  live,
}: {
  children: ReactNode
  live: RepoLiveState
}) {
  return (
    <RepoLayoutContext.Provider value={live}>
      {children}
    </RepoLayoutContext.Provider>
  )
}

export function useRepoLayout() {
  const live = use(RepoLayoutContext)
  if (!live) throw new Error('repository layout context is unavailable')
  return live
}
