import type { ClerkProp } from '@clerk/tanstack-react-start'

const resources = {
  client: null,
  organization: null,
  session: null,
  user: null,
}

const noop = () => {}

export const signedOutClerk = {
  __internal_lastEmittedResources: resources,
  __internal_updateProps: async () => {},
  addListener: () => noop,
  client: null,
  isSignedIn: false,
  load: async () => {},
  loaded: true,
  off: noop,
  on: (
    event: string,
    listener: (status: string) => void,
  ) => {
    if (event === 'status') {
      listener('ready')
    }
    return noop
  },
  organization: null,
  session: null,
  signOut: async () => {},
  status: 'ready',
  telemetry: { record: noop },
  user: null,
} as unknown as ClerkProp
