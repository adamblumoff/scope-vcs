import { loadAuthenticatedAccountForRequest } from '@/api/profile'
import { useAuth } from '@clerk/tanstack-react-start'
import { useRouterState } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import posthog, { type PostHog } from 'posthog-js'
import {
  type RefObject,
  useEffect,
  useRef,
  useSyncExternalStore,
} from 'react'
import { identityTransition, resolveAnalyticsIdentity } from './identity'
import { createPrivacyBoundary, pageViewProperties } from './privacy'
import { analyticsRouteForId } from './routes'

const defaultPostHogHost = 'https://us.i.posthog.com'

const loadAnalyticsIdentity = createServerFn({ method: 'GET' }).handler(
  async () => {
    const account = await loadAuthenticatedAccountForRequest()
    return account.user ? { scopeUserId: account.user.id } : null
  },
)

const analyticsClient = initializeAnalyticsClient()

export function AnalyticsRoot() {
  const hydrated = useSyncExternalStore(
    subscribeToHydration,
    getBrowserSnapshot,
    getServerSnapshot,
  )
  return hydrated && analyticsClient
    ? <AnalyticsRuntime client={analyticsClient} />
    : null
}

function AnalyticsRuntime({ client }: { client: PostHog }) {
  const { isLoaded, isSignedIn, userId } = useAuth()
  const routeId = useRouterState({
    select: (state) => state.matches.at(-1)?.routeId,
  })
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  })
  const capturedPage = useRef<string | null>(null)
  const identityKey = useRef<string | null>(null)
  const currentPage = useRef({ pathname, routeId })
  currentPage.current = { pathname, routeId }

  useEffect(() => {
    if (!isLoaded) return

    if (!isSignedIn || !userId) {
      applyIdentityTransition(client, null)
      identityKey.current = 'anonymous'
      captureCurrentPage(client, currentPage.current, capturedPage)
      return
    }

    identityKey.current = null
    let active = true
    void resolveAnalyticsIdentity(userId, loadAnalyticsIdentity)
      .then((identity) => {
        if (!active) return
        applyIdentityTransition(client, identity.scopeUserId)
        identityKey.current = identity.identityKey
        captureCurrentPage(client, currentPage.current, capturedPage)
      })

    return () => {
      active = false
    }
  }, [client, isLoaded, isSignedIn, userId])

  useEffect(() => {
    const expectedIdentityKey = isSignedIn && userId
      ? identifiedKey(userId)
      : 'anonymous'
    if (!isLoaded || identityKey.current !== expectedIdentityKey) return

    captureCurrentPage(client, { pathname, routeId }, capturedPage)
  }, [client, isLoaded, isSignedIn, pathname, routeId, userId])

  return null
}

function captureCurrentPage(
  client: PostHog,
  page: { pathname: string; routeId: string | undefined },
  capturedPage: RefObject<string | null>,
) {
  const route = analyticsRouteForId(page.routeId)
  if (!route) return

  const pageKey = `${page.routeId}:${page.pathname}`
  if (capturedPage.current === pageKey) return
  capturedPage.current = pageKey

  client.capture('$pageview', pageViewProperties(route, {
    origin: window.location.origin,
    referrer: document.referrer,
    search: window.location.search,
  }))
}

function identifiedKey(clerkUserId: string) {
  return `identified:${clerkUserId}`
}

function applyIdentityTransition(client: PostHog, scopeUserId: string | null) {
  const transition = identityTransition({
    currentDistinctId: client.get_distinct_id(),
    isSignedIn: Boolean(scopeUserId),
    persistedUserId: client.get_property('$user_id'),
    scopeUserId,
  })

  if (transition.kind === 'identify') {
    client.identify(transition.scopeUserId)
  } else if (transition.kind === 'reset') {
    client.reset()
  }
}

function initializeAnalyticsClient() {
  const config = analyticsConfig()
  if (!config) return null

  const client = posthog.init(config.token, {
    advanced_disable_flags: true,
    autocapture: false,
    before_send: createPrivacyBoundary(window.location.origin),
    capture_dead_clicks: false,
    capture_exceptions: false,
    capture_heatmaps: false,
    capture_pageleave: false,
    capture_pageview: false,
    capture_performance: false,
    disable_capture_url_hashes: true,
    disable_external_dependency_loading: true,
    disable_scroll_properties: true,
    disable_session_recording: true,
    person_profiles: 'identified_only',
    persistence: 'localStorage',
    rageclick: false,
    respect_dnt: true,
    save_campaign_params: false,
    save_referrer: false,
    api_host: config.host,
  })
  return client
}

function analyticsConfig() {
  if (!import.meta.env.PROD || typeof window === 'undefined') return null

  const token = import.meta.env.VITE_POSTHOG_PROJECT_TOKEN?.trim()
  const host = import.meta.env.VITE_POSTHOG_HOST?.trim() || defaultPostHogHost
  return token ? { host, token } : null
}

function subscribeToHydration() {
  return () => {}
}

function getBrowserSnapshot() {
  return true
}

function getServerSnapshot() {
  return false
}
