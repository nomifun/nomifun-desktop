/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';

import { creativeAssetClient } from '../assets/client';
import {
  createNomiPromptLibraryPort,
  creativePromptCatalogPort,
  PromptLibrarySidebar,
  type PromptLibraryItem,
  type PromptLibraryPort,
} from '../prompts';

export interface CreativePromptPickerProps {
  locale: string;
  applyLabel: string;
  enabled?: boolean;
  selectedId?: string | null;
  port?: PromptLibraryPort;
  onSelect(item: PromptLibraryItem): void;
}

/** The shared prompt-card interaction used by conversation and Canvas dialogs. */
const CreativePromptPicker: React.FC<CreativePromptPickerProps> = ({
  locale,
  applyLabel,
  enabled = true,
  selectedId,
  port,
  onSelect,
}) => {
  const productionPort = useMemo(
    () =>
      createNomiPromptLibraryPort({
        locale,
        assets: creativeAssetClient,
        catalog: creativePromptCatalogPort,
      }),
    [locale]
  );

  return (
    <PromptLibrarySidebar
      port={port ?? productionPort}
      enabled={enabled}
      selectedId={selectedId}
      showTagFilters={false}
      showHeader={false}
      visibleItemLimit={80}
      cardPresentation='visual-picker'
      applyLabel={applyLabel}
      onSelect={onSelect}
    />
  );
};

export default CreativePromptPicker;
