// The hosted service builds with VITE_EDITION=cloud; anything else is the
// self-hosted OpenVAIR console, which has no sign-up, sharing or public pages.
export const CLOUD = import.meta.env.VITE_EDITION === 'cloud'
export const PRODUCT = CLOUD ? 'VAIR' : 'OpenVAIR'
