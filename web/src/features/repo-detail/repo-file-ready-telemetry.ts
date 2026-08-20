import type { RepoParams } from '@/api/types'
import { createServerFn } from '@tanstack/react-start'
import { useEffect, useRef } from 'react'

type RepoFileReadyTiming = RepoParams & {
  duration_ms: number
  navigation_kind: 'client' | 'document'
  path: string
}

let documentNavigationClaimed = false

const reportRepoFileReady = createServerFn({ method: 'POST' })
  .validator(parseRepoFileReadyTiming)
  .handler(({ data }) => {
    console.info(JSON.stringify({ event: 'repo_file_ready', ...data }))
  })

export function useRepoFileReadyTiming({
  identity,
  owner,
  path,
  ready,
  repo,
}: {
  identity: string | null
  owner: string
  path: string | null
  ready: boolean
  repo: string
}) {
  const firstIdentity = useRef(true)
  const claimDocumentNavigation = useRef(false)
  const measurement = useRef<{
    identity: string | null
    navigationKind: 'client' | 'document'
    startedAt: number
  }>({ identity: null, navigationKind: 'document', startedAt: 0 })
  const reportedIdentity = useRef<string | null>(null)

  if (
    typeof performance !== 'undefined' &&
    measurement.current.identity !== identity
  ) {
    const navigationKind = firstIdentity.current
      ? initialNavigationKind()
      : 'client'
    measurement.current = {
      identity,
      navigationKind,
      startedAt: navigationKind === 'document' ? 0 : performance.now(),
    }
    claimDocumentNavigation.current = navigationKind === 'document'
    firstIdentity.current = false
  }

  useEffect(() => {
    if (claimDocumentNavigation.current) {
      documentNavigationClaimed = true
      claimDocumentNavigation.current = false
    }
  })

  useEffect(() => {
    if (!identity || !path || !ready || reportedIdentity.current === identity) {
      return
    }
    reportedIdentity.current = identity
    const frame = requestAnimationFrame(() => {
      const current = measurement.current
      if (current.identity !== identity) return
      void reportRepoFileReady({
        data: {
          duration_ms: Math.round(performance.now() - current.startedAt),
          navigation_kind: current.navigationKind,
          owner,
          path,
          repo,
        },
      }).catch(() => undefined)
    })
    return () => cancelAnimationFrame(frame)
  }, [identity, owner, path, ready, repo])
}

function initialNavigationKind(): 'client' | 'document' {
  if (documentNavigationClaimed || typeof window === 'undefined') {
    return 'client'
  }
  const [navigation] = performance.getEntriesByType('navigation')
  if (!navigation) return 'client'

  const documentUrl = new URL(navigation.name)
  const currentUrl = new URL(window.location.href)
  return documentUrl.origin === currentUrl.origin &&
    documentUrl.pathname === currentUrl.pathname &&
    documentUrl.search === currentUrl.search
    ? 'document'
    : 'client'
}

function parseRepoFileReadyTiming(input: RepoFileReadyTiming) {
  if (
    !input ||
    typeof input.owner !== 'string' ||
    typeof input.repo !== 'string' ||
    typeof input.path !== 'string' ||
    !Number.isFinite(input.duration_ms) ||
    input.duration_ms < 0 ||
    input.duration_ms > 300_000 ||
    (input.navigation_kind !== 'client' && input.navigation_kind !== 'document')
  ) {
    throw new Error('invalid repository file timing')
  }
  return input
}
