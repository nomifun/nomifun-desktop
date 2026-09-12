/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { Dispatch, PropsWithChildren, SetStateAction } from 'react';
import React, { useMemo, useState } from 'react';

/** Local state shared by a provider's descendants, not a controlled context.
 * The factory creates a fresh default for each mount. initialValue is used only
 * on mount; later prop changes must not overwrite updates from descendants.
 */
export function createContext<T>(initialize: () => T) {
  const Context = React.createContext<{
    value: T;
    setValue: Dispatch<SetStateAction<T>>;
  }>({
    value: initialize(),
    setValue() {
      console.warn('State context updated outside its provider');
    },
  });

  const useValue = () => React.useContext(Context).value;
  const useUpdate = () => React.useContext(Context).setValue;
  const Provider = ({ initialValue, children }: PropsWithChildren<{ initialValue?: T }>) => {
    const [value, setValue] = useState<T>(() => initialValue === undefined ? initialize() : initialValue);
    const context = useMemo(() => ({ value, setValue }), [value]);
    return <Context.Provider value={context}>{children}</Context.Provider>;
  };

  return [useValue, Provider, useUpdate] as const;
}
