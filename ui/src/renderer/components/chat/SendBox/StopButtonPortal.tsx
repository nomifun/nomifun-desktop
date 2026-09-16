import React, { createContext, useContext } from 'react';
import { createPortal } from 'react-dom';

// Layout only: the mounted SendBox retains its authoritative stop handler,
// pending state and duplicate-click guard when the chat is hidden by a surface.
export const StopButtonHostContext = createContext<HTMLElement | null>(null);

export function StopButtonPortal({ children }: { children: React.ReactNode }) {
  const host = useContext(StopButtonHostContext);
  return host ? createPortal(children, host) : <>{children}</>;
}
