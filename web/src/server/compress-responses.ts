import { PassThrough, Readable } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import type { ReadableStream as NodeReadableStream } from 'node:stream/web'
import { createBrotliCompress, createGzip } from 'node:zlib'
import { definePlugin } from 'nitro'

const COMPRESSIBLE_TYPES = [
  'application/javascript',
  'application/json',
  'application/xml',
  'image/svg+xml',
  'text/',
]

type Encoding = 'br' | 'gzip'

export default definePlugin((nitroApp) => {
  const fetch = nitroApp.fetch

  nitroApp.fetch = async (request) => {
    const response = await fetch(request)
    return compressResponse(request, response)
  }
})

function compressResponse(request: Request, response: Response) {
  const encoding = preferredEncoding(request.headers.get('accept-encoding'))

  if (
    !encoding ||
    request.method === 'HEAD' ||
    !response.body ||
    response.headers.has('content-encoding') ||
    response.headers.get('cache-control')?.includes('no-transform') ||
    response.status === 204 ||
    response.status === 304 ||
    !isCompressible(response.headers.get('content-type'))
  ) {
    return response
  }

  const headers = new Headers(response.headers)
  headers.set('content-encoding', encoding)
  headers.delete('content-length')
  appendVary(headers, 'Accept-Encoding')

  return new Response(compressBody(response.body, encoding), {
    headers,
    status: response.status,
    statusText: response.statusText,
  })
}

function compressBody(body: ReadableStream<Uint8Array>, encoding: Encoding) {
  const source = Readable.fromWeb(body as unknown as NodeReadableStream)
  const compressor = encoding === 'br' ? createBrotliCompress() : createGzip()
  const output = new PassThrough()

  pipeline(source, compressor, output).catch((error) => {
    output.destroy(error instanceof Error ? error : new Error(String(error)))
  })

  return Readable.toWeb(output) as unknown as BodyInit
}

function preferredEncoding(acceptEncoding: string | null): Encoding | null {
  if (!acceptEncoding) {
    return null
  }

  const weights = new Map<string, number>()
  for (const part of acceptEncoding.split(',')) {
    const [rawToken, ...rawParams] = part.trim().split(';')
    const token = rawToken?.trim().toLowerCase()
    if (!token || (token !== 'br' && token !== 'gzip' && token !== '*')) {
      continue
    }
    weights.set(token, parseQuality(rawParams))
  }

  const brotliQuality = acceptedQuality(weights, 'br')
  const gzipQuality = acceptedQuality(weights, 'gzip')

  if (brotliQuality <= 0 && gzipQuality <= 0) {
    return null
  }
  return brotliQuality >= gzipQuality ? 'br' : 'gzip'
}

function acceptedQuality(weights: Map<string, number>, encoding: Encoding) {
  return weights.get(encoding) ?? weights.get('*') ?? 0
}

function parseQuality(params: string[]) {
  const qParam = params
    .map((param) => param.trim().toLowerCase())
    .find((param) => param.startsWith('q='))

  if (!qParam) {
    return 1
  }

  const parsed = Number(qParam.slice(2))
  if (!Number.isFinite(parsed)) {
    return 0
  }
  return Math.max(0, Math.min(parsed, 1))
}

function isCompressible(contentType: string | null) {
  if (!contentType || contentType.startsWith('text/event-stream')) {
    return false
  }
  return COMPRESSIBLE_TYPES.some((type) => contentType.startsWith(type))
}

function appendVary(headers: Headers, value: string) {
  const current = headers.get('vary')
  if (!current) {
    headers.set('vary', value)
    return
  }

  const lowerValue = value.toLowerCase()
  const values = current.split(',').map((part) => part.trim().toLowerCase())
  if (!values.includes('*') && !values.includes(lowerValue)) {
    headers.set('vary', `${current}, ${value}`)
  }
}
