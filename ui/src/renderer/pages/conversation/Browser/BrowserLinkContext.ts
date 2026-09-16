import { createContext, useCallback, useContext } from 'react';
import { localBrowserLink, type LocalBrowserLink } from './localBrowserLink';

// Scoped to a mounted, native-capable conversation. Never resolve the current
// conversation from a global route or send a URL on a global event bus.
export const BrowserLinkContext = createContext<((link: LocalBrowserLink) => void) | null>(null);
export type BrowserLinkRequest = LocalBrowserLink & { id: number };

export function useBrowserLink(): (href: string) => boolean {
  const open = useContext(BrowserLinkContext);
  return useCallback((href: string) => {
    const link = localBrowserLink(href);
    if (!open || !link) return false;
    open(link);
    return true;
  }, [open]);
}
