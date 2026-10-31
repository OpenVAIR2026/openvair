import { defineConfig } from '@playwright/test'
import base from './playwright.overview.config'

export default defineConfig({
  ...base,
  testMatch: 'usage-*.spec.ts',
  webServer: {
    command: 'npm run build && npm exec vite preview -- --host 127.0.0.1 --port 4187 --strictPort',
    url: 'http://127.0.0.1:4187',
    reuseExistingServer: false,
    // The image build compiles Rust alongside this; a cold cache makes startup slow.
    timeout: 180_000,
    // Shows in the image build log why the server did not come up.
    stdout: 'pipe',
  },
})
