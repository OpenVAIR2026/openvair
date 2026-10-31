import { defineConfig } from '@playwright/test'
export default defineConfig({
  testDir: './tests',
  testMatch: 'model-baselines.spec.ts',
  use: { baseURL: 'http://127.0.0.1:4184', browserName: 'chromium' },
  webServer: { command: 'npm run dev -- --host 127.0.0.1 --port 4184 --strictPort', url: 'http://127.0.0.1:4184', reuseExistingServer: false },
})
