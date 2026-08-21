import assert from 'node:assert/strict'
import test from 'node:test'
import type { CaptureResult } from 'posthog-js'
import { pageViewProperties, sanitizeCapture } from './privacy'

const siteOrigin = 'https://scopevcs.com'

test('pageview properties replace raw paths and restrict campaign values', () => {
  const properties = pageViewProperties(
    { name: 'request_changes', path: '/repository/request/changes' },
    {
      origin: siteOrigin,
      referrer: 'https://search.example/private?q=secret',
      search: '?path=secret.rs&utm_source=docs&utm_medium=link&utm_campaign=launch%20private',
    },
  )

  assert.deepEqual(properties, {
    $current_url: 'https://scopevcs.com/repository/request/changes',
    $host: 'scopevcs.com',
    $pathname: '/repository/request/changes',
    $referrer: 'https://search.example',
    $referring_domain: 'search.example',
    route_name: 'request_changes',
    utm_medium: 'link',
    utm_source: 'docs',
  })
})

test('privacy boundary removes route parameters, query values, and unknown properties', () => {
  const capture: CaptureResult = {
    event: '$pageview',
    properties: {
      $current_url: 'https://scopevcs.com/adam/private-repo/code?path=secret.rs',
      $host: 'scopevcs.com',
      $geoip_disable: false,
      $pathname: '/adam/private-repo/code',
      $referrer: 'https://scopevcs.com/adam/private-repo',
      distinct_id: 'anonymous-id',
      owner: 'adam',
      path: 'secret.rs',
      repo: 'private-repo',
      request_id: 'req_secret',
      route_name: 'repository_code',
      token: 'phc_project',
      utm_source: 'docs',
    },
    uuid: 'event-id',
  }

  assert.deepEqual(sanitizeCapture(capture, siteOrigin), {
    event: '$pageview',
    properties: {
      $current_url: 'https://scopevcs.com/repository/code',
      $host: 'scopevcs.com',
      $geoip_disable: true,
      $pathname: '/repository/code',
      distinct_id: 'anonymous-id',
      route_name: 'repository_code',
      token: 'phc_project',
      utm_source: 'docs',
    },
    uuid: 'event-id',
  })
})

test('privacy boundary rejects every browser event outside the contract', () => {
  const capture = {
    event: '$autocapture',
    properties: { distinct_id: 'anonymous-id' },
    uuid: 'event-id',
  }
  assert.equal(sanitizeCapture(capture, siteOrigin), null)
})

test('identify keeps identity linkage but drops person and URL properties', () => {
  const capture: CaptureResult = {
    $set: { email: 'private@example.com' },
    event: '$identify',
    properties: {
      $anon_distinct_id: 'anonymous-id',
      $current_url: 'https://scopevcs.com/adam/private-repo',
      distinct_id: 'scope_usr_123',
      email: 'private@example.com',
      token: 'phc_project',
    },
    uuid: 'event-id',
  }

  assert.deepEqual(sanitizeCapture(capture, siteOrigin), {
    event: '$identify',
    properties: {
      $anon_distinct_id: 'anonymous-id',
      $geoip_disable: true,
      distinct_id: 'scope_usr_123',
      token: 'phc_project',
    },
    uuid: 'event-id',
  })
})
