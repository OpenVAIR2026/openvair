import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: './tests',
  testMatch: 'overview.spec.ts',
  outputDir: './test-results',
  fullyParallel: true,
  workers: 2,
  use: {
    baseURL: 'http://127.0.0.1:4187',
    viewport: { width: 1440, height: 1000 },
    timezoneId: 'Asia/Shanghai',
    reducedMotion: 'reduce',
  },
  webServer: {
    command: 'npm run dev -- --host 127.0.0.1 --port 4187 --strictPort',
    url: 'http://127.0.0.1:4187',
    reuseExistingServer: false,
  },
})
