/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_NOMIFUN_PRODUCT_MODE?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
