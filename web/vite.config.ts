import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import tailwindcss from '@tailwindcss/vite'
import viteReact from '@vitejs/plugin-react'
import path from 'node:path'
import { nitro } from 'nitro/vite'
import { defineConfig } from 'vite'

export default defineConfig({
  server: {
    allowedHosts: ['adam-blumoff-surface-book-2.tail93abb2.ts.net'],
    port: 3000,
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
    dedupe: ['react', 'react-dom'],
  },
  plugins: [
    tailwindcss(),
    tanstackStart({
      srcDirectory: 'src',
      server: {
        build: {
          inlineCss: false,
        },
      },
    }),
    viteReact(),
    nitro({
      compressPublicAssets: {
        brotli: true,
        gzip: true,
      },
      plugins: ['./src/server/compress-responses.ts'],
    }),
  ],
})
