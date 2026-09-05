export type NomiFunProductMode = 'default' | 'sales';

export const resolveProductMode = (value: unknown): NomiFunProductMode =>
  typeof value === 'string' && value.trim().toLowerCase() === 'sales' ? 'sales' : 'default';

export const productMode = resolveProductMode(import.meta.env.VITE_NOMIFUN_PRODUCT_MODE);
export const isSalesProductMode = productMode === 'sales';
export const authenticatedHomePath = isSalesProductMode ? '/sales' : '/guid';

