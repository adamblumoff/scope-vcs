import assert from 'node:assert/strict'
import test from 'node:test'
import { createBoundedCache } from './bounded-cache'

test('evicts the least-recently-used entry at the entry limit', () => {
  const cache = createBoundedCache<string, number>({ maxEntries: 2 })
  cache.set('first', 1)
  cache.set('second', 2)

  assert.equal(cache.get('first'), 1)
  cache.set('third', 3)

  assert.equal(cache.peek('first'), 1)
  assert.equal(cache.peek('second'), undefined)
  assert.equal(cache.peek('third'), 3)
})

test('peek reads without extending an entry lifetime', () => {
  const cache = createBoundedCache<string, number>({ maxEntries: 2 })
  cache.set('first', 1)
  cache.set('second', 2)

  assert.equal(cache.peek('first'), 1)
  cache.set('third', 3)

  assert.equal(cache.peek('first'), undefined)
  assert.equal(cache.peek('second'), 2)
})

test('accounts for replacements and evicts at the weight limit', () => {
  const cache = createBoundedCache<string, string>({
    maxEntries: 10,
    maxWeight: 5,
    weightOf: (value) => value.length,
  })
  cache.set('first', '1234')
  cache.set('second', '5')
  cache.set('first', '12')

  assert.deepEqual(cache.stats(), { entries: 2, totalWeight: 3 })

  cache.set('third', '3456')
  assert.equal(cache.peek('second'), undefined)
  assert.equal(cache.peek('first'), undefined)
  assert.equal(cache.peek('third'), '3456')
  assert.deepEqual(cache.stats(), { entries: 1, totalWeight: 4 })
})

test('evicts an overweight value after it becomes the only entry', () => {
  const cache = createBoundedCache<string, string>({
    maxEntries: 10,
    maxWeight: 5,
    weightOf: (value) => value.length,
  })
  cache.set('small', '12')
  cache.set('overweight', '123456')

  assert.equal(cache.peek('small'), undefined)
  assert.equal(cache.peek('overweight'), undefined)
  assert.deepEqual(cache.stats(), { entries: 0, totalWeight: 0 })
})

test('clear removes values and resets weight accounting', () => {
  const cache = createBoundedCache<string, string>({
    maxEntries: 2,
    weightOf: (value) => value.length,
  })
  cache.set('first', '123')
  cache.clear()

  assert.equal(cache.get('first'), undefined)
  assert.deepEqual(cache.stats(), { entries: 0, totalWeight: 0 })
})
