import { defineConfig } from '@playwright/test'
export default defineConfig({
  testDir: './tests',
  testMatch: 'quality-monitoring.spec.ts',
  use: { baseURL: 'http://127.0.0.1:4183', browserName: 'chromium' },
  webServer: { command: 'npm run dev -- --host 127.0.0.1 --port 4183 --strictPort', url: 'http://127.0.0.1:4183', reuseExistingServer: false },
})
