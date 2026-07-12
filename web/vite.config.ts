import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import tailwindcss from '@tailwindcss/vite'
import viteReact from '@vitejs/plugin-react'
import path from 'node:path'
import { nitro } from 'nitro/vite'
import { defineConfig, loadEnv } from 'vite'

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, __dirname, '')
  const allowedHost = env.SCOPE_WEB_ALLOWED_HOST?.trim()

  return {
    server: {
      allowedHosts: allowedHost ? [allowedHost] : [],
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
  }
})
