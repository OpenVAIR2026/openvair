/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_EDITION?: 'cloud' | 'community'
}

interface ImportMeta {
  readonly env: ImportMetaEnv
}
