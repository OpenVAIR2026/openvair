import { defineConfig } from '@playwright/test'
import base from './playwright.overview.config'

export default defineConfig({ ...base, testMatch: ['model-pricing.spec.ts', 'model-add.spec.ts', 'model-discover.spec.ts', 'key-cost-policy.spec.ts'] })
