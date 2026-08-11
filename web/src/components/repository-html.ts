const HTML_DOCTYPE = /^\s*<!doctype\s+html[^>]*>/i
const HTML_META_START = /<meta(?=[\s/>])/gi

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
  const authoredDocument = source
    .replace(HTML_DOCTYPE, '')
    .replace(HTML_META_START, '&lt;meta')
  return `<!doctype html>${REPOSITORY_HTML_POLICY}${authoredDocument}`
}
