/// <reference types="vite/client" />
import {
  HeadContent,
  Outlet,
  Scripts,
  createRootRouteWithContext,
} from '@tanstack/react-router'
import type { QueryClient } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import appCss from '@/styles.css?url'

const SITE_URL = 'https://pfs.grok.me'
const TITLE =
  'ProofShip — native coding agents, on-device wallets, and X Layer deploy'
const DESCRIPTION =
  'ProofShip is a fast, native desktop app for Amp, Claude Code, Codex, Cursor, Grok, Kimi, OpenCode, and Pi. Preview, sign, and deploy to OKX X Layer from the same window — no browser wallet extension required.'
const OG_IMAGE = `${SITE_URL}/og-icon.png`
const JSON_LD = JSON.stringify({
  '@context': 'https://schema.org',
  '@type': 'SoftwareApplication',
  name: 'ProofShip',
  applicationCategory: 'DeveloperApplication',
  operatingSystem: 'macOS, Linux, Windows',
  url: SITE_URL,
  downloadUrl: 'https://github.com/DaviRain-Su/proof_ship/releases/latest',
  license: 'https://github.com/DaviRain-Su/proof_ship/blob/dev/LICENSE',
  description: DESCRIPTION,
  offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
})

export const Route = createRootRouteWithContext<{
  queryClient: QueryClient
}>()({
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      { name: 'viewport', content: 'width=device-width, initial-scale=1' },
      { title: TITLE },
      { name: 'description', content: DESCRIPTION },
      { name: 'robots', content: 'index, follow' },
      { name: 'author', content: 'ProofShip' },
      { name: 'keywords', content: 'ProofShip, coding agents, X Layer, OKX, EVM, GPUI, Rust, Claude Code, Codex, wallet, deploy' },
      { property: 'og:title', content: TITLE },
      { property: 'og:description', content: DESCRIPTION },
      { property: 'og:type', content: 'website' },
      { property: 'og:site_name', content: 'ProofShip' },
      { property: 'og:locale', content: 'en_US' },
      { property: 'og:url', content: SITE_URL },
      { property: 'og:image', content: OG_IMAGE },
      { property: 'og:image:alt', content: 'ProofShip app icon' },
      { name: 'twitter:card', content: 'summary_large_image' },
      { name: 'twitter:title', content: TITLE },
      { name: 'twitter:description', content: DESCRIPTION },
      { name: 'twitter:image', content: OG_IMAGE },
      {
        name: 'theme-color',
        media: '(prefers-color-scheme: light)',
        content: '#ffffff',
      },
      {
        name: 'theme-color',
        media: '(prefers-color-scheme: dark)',
        content: '#1e1e1e',
      },
    ],
    links: [
      { rel: 'stylesheet', href: appCss },
      { rel: 'canonical', href: SITE_URL },
      { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/favicon.png' },
      { rel: 'apple-touch-icon', sizes: '180x180', href: '/apple-touch-icon.png' },
    ],
    scripts: [
      { type: 'application/ld+json', children: JSON_LD },
      {
        // Mirror the system color scheme onto <html> before first paint.
        children: `try{var m=matchMedia('(prefers-color-scheme: dark)'),d=document.documentElement,s=function(){d.classList.toggle('dark',m.matches)};s();m.addEventListener('change',s)}catch(e){}`,
      },
      // Analytics, production builds only.
      ...(import.meta.env.PROD
        ? [
            {
              defer: true,
              src: 'https://u.egoist.dev/script.js',
              'data-website-id': '5dc2da71-cd6e-4862-8d60-e1cfb782f54f',
            },
          ]
        : []),
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
    <html lang="en" suppressHydrationWarning>
      <head>
        <HeadContent />
      </head>
      <body>
        {children}
        <Scripts />
      </body>
    </html>
  )
}
