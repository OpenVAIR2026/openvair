import { defineConfig } from '@playwright/test'
import base from './playwright.overview.config'

export default defineConfig({
  ...base,
  testMatch: 'key-*.spec.ts',
})
