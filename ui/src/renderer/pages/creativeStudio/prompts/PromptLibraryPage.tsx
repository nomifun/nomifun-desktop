/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

import PromptLibrarySurface from './PromptLibrarySurface';
import type { PromptLibrarySurfaceProps } from './PromptLibrarySurface';
import type { PromptLibraryPort } from './types';
import { usePromptLibrary } from './usePromptLibrary';

export interface PromptLibraryPageProps
  extends Omit<
    PromptLibrarySurfaceProps,
    'variant' | 'items' | 'loading' | 'refreshing' | 'error' | 'invalidCount' | 'onRetry'
  > {
  port: PromptLibraryPort;
  enabled?: boolean;
}

export const PromptLibraryPage: React.FC<PromptLibraryPageProps> = ({
  port,
  enabled = true,
  ...props
}) => {
  const state = usePromptLibrary(port, { enabled });
  return (
    <PromptLibrarySurface
      {...props}
      variant='page'
      items={state.items}
      loading={state.loading}
      refreshing={state.refreshing}
      error={state.error}
      invalidCount={state.invalidCount}
      onRetry={() => void state.reload()}
    />
  );
};

export default PromptLibraryPage;
