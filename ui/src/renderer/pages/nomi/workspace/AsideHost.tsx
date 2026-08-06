/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { createContext, useContext, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';

/**
 * The right-hand detail pane is owned by whichever tab needs it, but it must
 * render as a flex SIBLING of the workspace column — not nested inside the
 * tab's scroll area. `AsideHost` is the shell-level landing zone; tabs portal
 * their `<ContentAside>` into it with `useAsidePortal`.
 *
 * The host div uses `display: contents`, so the portalled panel becomes a direct
 * flex child of the three-column row rather than being boxed by a wrapper.
 */
const AsideHostContext = createContext<HTMLElement | null>(null);

export const AsideHost: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [host, setHost] = useState<HTMLDivElement | null>(null);
  const value = useMemo(() => host, [host]);
  return (
    <AsideHostContext.Provider value={value}>
      {children}
      <div ref={setHost} style={{ display: 'contents' }} />
    </AsideHostContext.Provider>
  );
};

/**
 * Portal `node` into the shell's aside slot. Returns null until the host mounts,
 * so callers can render it unconditionally.
 */
export const useAsidePortal = (node: React.ReactNode): React.ReactPortal | null => {
  const host = useContext(AsideHostContext);
  if (!host) return null;
  return createPortal(node, host);
};
