import type { CaptureResult, Properties, Property } from 'posthog-js'
import {
  analyticsRouteForName,
  type AnalyticsRoute,
} from './routes'

const transportPropertyNames = [
  '$anon_distinct_id',
  '$device_id',
  '$geoip_disable',
  '$is_identified',
  '$lib',
  '$lib_version',
  '$process_person_profile',
  '$session_id',
  '$user_id',
  '$window_id',
  'distinct_id',
  'token',
] as const

const campaignPropertyNames = [
  'utm_campaign',
  'utm_medium',
  'utm_source',
] as const

const campaignValuePattern = /^[a-zA-Z0-9._-]{1,80}$/

export type PageViewContext = {
  origin: string
  referrer: string
  search: string
}

export function pageViewProperties(
  route: AnalyticsRoute,
  context: PageViewContext,
): Properties {
  const properties: Properties = {
    $current_url: `${context.origin}${route.path}`,
    $host: new URL(context.origin).host,
    $pathname: route.path,
    route_name: route.name,
  }
  const referrer = externalReferrer(context.referrer, context.origin)
  if (referrer) {
    properties.$referrer = referrer.origin
    properties.$referring_domain = referrer.host
  }

  const search = new URLSearchParams(context.search)
  for (const propertyName of campaignPropertyNames) {
    const value = search.get(propertyName)
    if (value && campaignValuePattern.test(value)) {
      properties[propertyName] = value
    }
  }

  return properties
}

export function createPrivacyBoundary(origin: string) {
  const siteOrigin = new URL(origin).origin
  return (capture: CaptureResult | null) => sanitizeCapture(capture, siteOrigin)
}

export function sanitizeCapture(
  capture: CaptureResult | null,
  siteOrigin: string,
): CaptureResult | null {
  if (!capture) return null

  if (capture.event === '$identify') {
    return withoutPersonMutations(
      capture,
      transportProperties(capture.properties),
    )
  }

  if (capture.event !== '$pageview') return null

  const routeName = capture.properties.route_name
  if (typeof routeName !== 'string') return null

  const route = analyticsRouteForName(routeName)
  if (!route) return null

  const origin = new URL(siteOrigin).origin
  const properties: Properties = {
    ...transportProperties(capture.properties),
    $current_url: `${origin}${route.path}`,
    $host: new URL(origin).host,
    $pathname: route.path,
    route_name: route.name,
  }

  copyCampaignProperties(capture.properties, properties)
  copyExternalReferrer(capture.properties, properties, origin)

  return withoutPersonMutations(capture, properties)
}

function withoutPersonMutations(
  capture: CaptureResult,
  properties: Properties,
) {
  const sanitized = { ...capture, properties }
  delete sanitized.$set
  delete sanitized.$set_once
  delete sanitized.$unset
  return sanitized
}

function transportProperties(properties: Properties) {
  const allowed: Properties = {}
  for (const propertyName of transportPropertyNames) {
    const value = properties[propertyName]
    if (isPostHogProperty(value)) allowed[propertyName] = value
  }
  return allowed
}

function copyCampaignProperties(source: Properties, target: Properties) {
  for (const propertyName of campaignPropertyNames) {
    const value = source[propertyName]
    if (typeof value === 'string' && campaignValuePattern.test(value)) {
      target[propertyName] = value
    }
  }
}

function copyExternalReferrer(
  source: Properties,
  target: Properties,
  siteOrigin: string,
) {
  const value = source.$referrer
  if (typeof value !== 'string') return

  const referrer = externalReferrer(value, siteOrigin)
  if (!referrer) return

  target.$referrer = referrer.origin
  target.$referring_domain = referrer.host
}

function externalReferrer(referrer: string, siteOrigin: string) {
  if (!referrer) return null

  try {
    const parsed = new URL(referrer)
    return parsed.origin === new URL(siteOrigin).origin ? null : parsed
  } catch {
    return null
  }
}

function isPostHogProperty(value: Property | undefined): value is Property {
  return value !== undefined
}
