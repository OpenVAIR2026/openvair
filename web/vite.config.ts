import { defineConfig, loadEnv } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig(({ mode }) => {
  const root = new URL('.', import.meta.url).pathname
  const cloud = loadEnv(mode, root, 'VITE_').VITE_EDITION === 'cloud'
  return {
    plugins: [
      react(),
      tailwindcss(),
      // The page title is in index.html, before any script runs.
      { name: 'edition-title', transformIndexHtml: (html: string) => cloud ? html : html.replace('<title>VAIR ', '<title>OpenVAIR ') },
    ],
    resolve: { alias: { '@': new URL('./src', import.meta.url).pathname } },
    server: { proxy: { '/api': 'http://127.0.0.1:8080', '/v1': 'http://127.0.0.1:8080' } },
  }
})
