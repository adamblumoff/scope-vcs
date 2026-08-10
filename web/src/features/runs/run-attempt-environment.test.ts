import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type { RepoRunCache } from '@/api/types'
import { RunAttemptEnvironment } from './run-attempt-environment'

const image = `registry/scope@sha256:${'c'.repeat(64)}`
const caches: RepoRunCache[] = [
  {
    name: 'cargo',
    path: '/scope/cache/cargo',
    observation: {
      workflow_path: '/.scope/runs/checks.yml',
      job_key: 'backend',
      identity_digest: 'a'.repeat(64),
      preparation: { kind: 'cold', reason: 'metadata-missing' },
      prepare_ms: 12,
      final_state: 'ready',
      finalize_ms: 8,
    },
  },
  {
    name: 'target',
    path: '/workspace/target',
    observation: null,
  },
]

describe('run attempt environment', () => {
  it('renders compact inline facts without presenting them as cards', () => {
    const html = renderToStaticMarkup(createElement(RunAttemptEnvironment, {
      caches,
      pinnedContainerImage: image,
    }))

    assert.match(html, /aria-label="Execution environment"/)
    assert.match(html, /Environment/)
    assert.match(html, /1 cold · 1 not reported · prepared in 12ms/)
    assert.match(html, /No reusable entry for this identity/)
    assert.match(html, /Cache facts were not reported for this attempt/)
    assert.match(html, new RegExp(`title="${image}"`))
    assert.doesNotMatch(html, /rounded|shadow/)
  })
})
