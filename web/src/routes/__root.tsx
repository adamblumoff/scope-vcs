import { forceSignedOut, signedOutPublishableKey } from '@/auth-mode'
import { signedOutClerk } from '@/lib/signed-out-clerk'
import { ClerkProvider } from '@clerk/tanstack-react-start'
import {
  HeadContent,
  Outlet,
  Scripts,
  createRootRoute,
} from '@tanstack/react-router'
import type { ReactNode } from 'react'
import { Toaster } from 'sonner'
import { scopeClerkAppearance } from '../clerk-appearance'
import '../styles.css'

export const Route = createRootRoute({
  head: () => ({
    links: [
      {
        rel: 'icon',
        type: 'image/svg+xml',
        href: '/favicon.svg',
      },
    ],
    meta: [
      { charSet: 'utf-8' },
      {
        name: 'viewport',
        content: 'width=device-width, initial-scale=1',
      },
      {
        title: 'Scope',
      },
      {
        name: 'description',
        content: 'Permissioned source-control projections.',
      },
      {
        property: 'og:type',
        content: 'website',
      },
      {
        property: 'og:site_name',
        content: 'Scope',
      },
      {
        property: 'og:title',
        content: 'Scope',
      },
      {
        property: 'og:description',
        content: 'Permissioned source-control projections.',
      },
      {
        property: 'og:image',
        content: 'https://scopevcs.com/brand/scope-social.png',
      },
      {
        property: 'og:image:width',
        content: '1200',
      },
      {
        property: 'og:image:height',
        content: '630',
      },
      {
        property: 'og:image:alt',
        content: 'Scope',
      },
      {
        name: 'twitter:card',
        content: 'summary_large_image',
      },
      {
        name: 'twitter:title',
        content: 'Scope',
      },
      {
        name: 'twitter:description',
        content: 'Permissioned source-control projections.',
      },
      {
        name: 'twitter:image',
        content: 'https://scopevcs.com/brand/scope-social.png',
      },
      {
        name: 'twitter:image:alt',
        content: 'Scope',
      },
    ],
  }),
  component: RootComponent,
})

function RootComponent() {
  return (
    <RootDocument>
      <Outlet />
    </RootDocument>
  )
}

function RootDocument({ children }: { children: ReactNode }) {
  return (
    <html
      className="dark"
      lang="en"
      style={{ colorScheme: 'dark' }}
      suppressHydrationWarning
    >
      <head>
        <HeadContent />
        <script
          dangerouslySetInnerHTML={{
            __html: `(function(){try{var s=localStorage.getItem('scope-theme');var dark=s?s==='dark':true;var e=document.documentElement;e.classList.toggle('dark',dark);e.style.colorScheme=dark?'dark':'light';}catch(_){}})();`,
          }}
        />
      </head>
      <body>
        <ClerkProvider
          appearance={scopeClerkAppearance}
          {...(forceSignedOut
            ? {
                Clerk: signedOutClerk,
                publishableKey: signedOutPublishableKey,
              }
            : {})}
        >
          {children}
          <Toaster richColors position="bottom-right" />
          <Scripts />
        </ClerkProvider>
      </body>
    </html>
  )
}
