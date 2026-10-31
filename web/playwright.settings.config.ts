import { defineConfig } from '@playwright/test'
import base from './playwright.overview.config'

// A dedicated port lets this suite run while other suites hold 4187.
export default defineConfig({
  ...base,
  testMatch: ['account-settings.spec.ts', 'signup-verification.spec.ts'],
  use: { ...base.use, baseURL: 'http://127.0.0.1:4189' },
  webServer: { command: 'npm run dev -- --host 127.0.0.1 --port 4189 --strictPort', url: 'http://127.0.0.1:4189', reuseExistingServer: false },
})
