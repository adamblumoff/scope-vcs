const HTML_DOCTYPE = /^\s*<!doctype\s+html[^>]*>/i

export const REPOSITORY_HTML_CONTENT_SECURITY_POLICY = [
  "default-src 'none'",
  "base-uri 'none'",
  "connect-src 'none'",
  'font-src data:',
  "form-action 'none'",
  "frame-src 'none'",
  'img-src data:',
  'media-src data:',
  "object-src 'none'",
  "script-src 'none'",
  "style-src 'unsafe-inline'",
].join('; ')

const REPOSITORY_HTML_POLICY = `<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta http-equiv="Content-Security-Policy" content="${REPOSITORY_HTML_CONTENT_SECURITY_POLICY}"><meta http-equiv="x-dns-prefetch-control" content="off"><base target="_blank">`

export function isRepositoryHtmlPath(path: string) {
  const fileName = path.replace(/^\/+/, '').split('/').at(-1) ?? ''
  return /\.html$/i.test(fileName)
}

export function repositoryHtmlDocument(source: string) {
  const authoredDocument = stripAuthoredMetaElements(
    source.replace(HTML_DOCTYPE, ''),
  )
  return `<!doctype html>${REPOSITORY_HTML_POLICY}${authoredDocument}`
}

function stripAuthoredMetaElements(source: string) {
  const metaStart = /<meta(?=[\t\n\f\r />])/gi
  let output = ''
  let offset = 0

  for (const match of source.matchAll(metaStart)) {
    const start = match.index
    if (start < offset) continue

    output += source.slice(offset, start)
    const end = htmlTagEnd(source, start + match[0].length)
    if (end === undefined) return output
    offset = end + 1
  }

  return output + source.slice(offset)
}

function htmlTagEnd(source: string, offset: number) {
  let quote: '"' | "'" | undefined

  for (let index = offset; index < source.length; index += 1) {
    const character = source[index]
    if (quote) {
      if (character === quote) quote = undefined
    } else if (character === '"' || character === "'") {
      quote = character
    } else if (character === '>') {
      return index
    }
  }

  return undefined
}
