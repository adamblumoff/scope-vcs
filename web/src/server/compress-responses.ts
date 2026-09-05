import { definePlugin } from 'nitro'
import { compressResponse } from './response-compression'

export default definePlugin((nitroApp) => {
  const fetch = nitroApp.fetch
  nitroApp.fetch = async (request) =>
    compressResponse(request, await fetch(request))
})
