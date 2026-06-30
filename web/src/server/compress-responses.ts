import { Readable } from 'node:stream'
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

  const source = Readable.fromWeb(
    response.body as unknown as NodeReadableStream,
  )
  const compressed = source.pipe(
    encoding === 'br' ? createBrotliCompress() : createGzip(),
  )

  return new Response(Readable.toWeb(compressed) as unknown as BodyInit, {
    headers,
    status: response.status,
    statusText: response.statusText,
  })
}

function preferredEncoding(acceptEncoding: string | null) {
  if (!acceptEncoding) {
    return null
  }
  const accepted = acceptEncoding.toLowerCase()
  if (accepted.includes('br')) {
    return 'br'
  }
  if (accepted.includes('gzip')) {
    return 'gzip'
  }
  return null
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
