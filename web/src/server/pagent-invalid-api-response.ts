import {
  setInvalidApiResponseObserver,
  type InvalidApiResponseError,
} from '@/api/http'
import { definePlugin } from 'nitro'
import { createPagent, defineEvent } from 'pagent'
import { invalidApiResponseGroup } from './pagent-invalid-api-response-group'

type InvalidApiResponseContext = {
  contentType: string | null
  failureClass: InvalidApiResponseError['failureClass']
  issuePath?: string
  method: string
  path: string
  status: number
}

const invalidApiResponse = defineEvent<InvalidApiResponseContext>({
  name: 'scope.web.api.invalid_response',
  enabledIn: ['staging'],
  investigation: { cooldownMs: 5 * 60_000 },
})

const enabled = process.env.PAGENT_ENABLED === 'true'
const environment = enabled ? requiredEnvironment('PAGENT_ENV') : undefined

const pagent = createPagent({
  enabled,
  environment,
  ...(enabled
    ? {
        encryption: {
          keyId: requiredEnvironment('PAGENT_ENCRYPTION_KEY_ID'),
          key: requiredEnvironment('PAGENT_ENCRYPTION_KEY'),
        },
        endpoint: {
          url: requiredEnvironment('PAGENT_ENDPOINT_URL'),
          token: requiredEnvironment('PAGENT_SOURCE_TOKEN'),
        },
      }
    : {}),
  onDeliveryError: (error) => {
    console.error('[pagent] invalid API response delivery failed', {
      code: error.code,
      statusCode: error.statusCode,
    })
  },
})

const reportInvalidApiResponse = pagent.observe(
  (error: InvalidApiResponseError) => error,
  {
    event: invalidApiResponse,
    on: 'result',
    triggerWhen: () => true,
    group: ({ result }) => invalidApiResponseGroup(result),
    context: ({ result }) => ({
      contentType: result.contentType,
      failureClass: result.failureClass,
      issuePath: result.issuePath,
      method: result.requestMethod,
      path: result.requestPath,
      status: result.status,
    }),
  },
)

function observeInvalidApiResponse(error: InvalidApiResponseError) {
  reportInvalidApiResponse(error)
}

export default definePlugin((nitroApp) => {
  setInvalidApiResponseObserver(observeInvalidApiResponse)
  nitroApp.hooks.hook('close', () => pagent.flush())
})

function requiredEnvironment(name: string) {
  const value = process.env[name]?.trim()
  if (!value) throw new Error(`${name} is required when Pagent is enabled.`)
  return value
}
