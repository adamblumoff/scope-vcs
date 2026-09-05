import assert from 'node:assert/strict'
import { brotliDecompressSync, gunzipSync } from 'node:zlib'
import test from 'node:test'
import { compressResponse } from './response-compression'

const source = JSON.stringify({ text: 'Scope source control\n'.repeat(200) })

function request(encoding: string, method = 'GET') {
  return new Request('https://scope.test/data', {
    headers: { 'accept-encoding': encoding }, method,
  })
}

function response(headers: Record<string, string> = {}) {
  return new Response(source, {
    headers: { 'content-type': 'application/json', ...headers },
  })
}

test('negotiates encodings, round trips content and preserves Vary', async () => {
  for (const [accepted, expected] of [
    ['gzip, br', 'br'], ['br;q=0.5,gzip;q=1', 'gzip'], ['*', 'br'],
    ['br;q=0,gzip;q=0', null], ['br;q=invalid', null],
  ] as const) {
    const result = compressResponse(request(accepted), response({ vary: 'Cookie' }))
    assert.equal(result.headers.get('content-encoding'), expected)
    assert.equal(result.headers.get('vary'), 'Cookie, Accept-Encoding')
    const bytes = Buffer.from(await result.arrayBuffer())
    const decoded = expected === 'br' ? brotliDecompressSync(bytes)
      : expected === 'gzip' ? gunzipSync(bytes) : bytes
    assert.equal(decoded.toString(), source)
  }
})

test('leaves known tiny bodies and excluded responses alone', () => {
  for (const headers of [
    { 'content-length': '123' },
    { 'content-encoding': 'gzip' },
    { 'cache-control': 'no-transform' },
    { 'content-type': 'text/event-stream' },
    { 'content-type': 'application/octet-stream' },
  ] as Array<Record<string, string>>) {
    const original = response(headers)
    assert.equal(compressResponse(request('br'), original), original)
  }
  const original = response()
  assert.equal(compressResponse(request('br', 'HEAD'), original), original)
  for (const status of [204, 304]) {
    const empty = new Response(null, { status })
    assert.equal(compressResponse(request('br'), empty), empty)
  }
})

test('starts an unknown-length stream without waiting for the source to finish', async () => {
  let close: (() => void) | undefined
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(new TextEncoder().encode(source))
      close = () => controller.close()
    },
  })
  const result = compressResponse(request('br'), new Response(body, {
    headers: { 'content-type': 'text/html' },
  }))
  assert.equal(result.headers.get('content-encoding'), 'br')
  close?.()
  assert.equal(brotliDecompressSync(Buffer.from(await result.arrayBuffer())).toString(), source)
})
