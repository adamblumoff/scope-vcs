import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  buildRunJobGraph,
  orderJobsByDependency,
} from './run-job-graph-model'

function job(key: string, needs: string[] = []) {
  return { job: { key, needs } }
}

describe('run job graph layout', () => {
  it('builds deterministic topological layers and dependency edges', () => {
    const layout = buildRunJobGraph([
      job('integration', ['backend', 'web']),
      job('web'),
      job('backend'),
      job('release', ['integration']),
    ])

    assert.deepEqual(
      layout.nodes.map(({ key, layer }) => ({ key, layer })),
      [
        { key: 'backend', layer: 0 },
        { key: 'web', layer: 0 },
        { key: 'integration', layer: 1 },
        { key: 'release', layer: 2 },
      ],
    )
    assert.deepEqual(
      layout.edges.map(({ from, to }) => `${from}->${to}`),
      ['backend->integration', 'web->integration', 'integration->release'],
    )
  })

  it('rejects impossible persisted graphs', () => {
    assert.throws(() => buildRunJobGraph([job('checks', ['missing'])]), /missing/)
    assert.throws(
      () => buildRunJobGraph([job('one', ['two']), job('two', ['one'])]),
      /cycle/,
    )
  })

  it('keeps edge identities distinct for kebab-case job keys', () => {
    const layout = buildRunJobGraph([
      job('a-b'),
      job('a'),
      job('c', ['a-b']),
      job('b-c', ['a']),
    ])

    assert.equal(new Set(layout.edges.map((edge) => edge.key)).size, 2)
  })

  it('routes long dependencies through a bounded corridor', () => {
    const jobs = Array.from({ length: 20 }, (_, index) =>
      job(`job-${index}`, Array.from({ length: index }, (_item, need) => `job-${need}`)))
    const layout = buildRunJobGraph(jobs)
    const longEdge = layout.edges.find(
      (edge) => edge.from === 'job-0' && edge.to === 'job-19',
    )

    assert.match(longEdge?.path ?? '', / L /)
    assert.ok(layout.height < 300)
  })
})

describe('run job ordering', () => {
  it('orders the job strip by dependency, not alphabetically', () => {
    const jobs = [
      { job: { key: 'build', needs: [] } },
      { job: { key: 'deploy', needs: ['test'] } },
      { job: { key: 'test', needs: ['build'] } },
    ]

    assert.deepEqual(
      orderJobsByDependency(jobs).map(({ job }) => job.key),
      ['build', 'test', 'deploy'],
    )
  })

  it('keeps the given order when the graph cannot be laid out', () => {
    const jobs = [
      { job: { key: 'a', needs: ['b'] } },
      { job: { key: 'b', needs: ['a'] } },
    ]

    assert.deepEqual(
      orderJobsByDependency(jobs).map(({ job }) => job.key),
      ['a', 'b'],
    )
  })
})
