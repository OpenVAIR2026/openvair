import { defineConfig } from '@playwright/test'
export default defineConfig({
  testDir: './tests',
  testMatch: 'public-verify-settings.spec.ts',
  use: { baseURL: 'http://127.0.0.1:4185', browserName: 'chromium' },
  webServer: { command: 'npm run dev -- --host 127.0.0.1 --port 4185 --strictPort', url: 'http://127.0.0.1:4185', reuseExistingServer: false },
})
