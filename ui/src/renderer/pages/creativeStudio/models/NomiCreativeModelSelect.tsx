/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';

import CreativeModelSelect, { type CreativeModelSelectProps } from './CreativeModelSelect';
import { useNomiCreativeModelCatalog } from './useNomiCreativeModelCatalog';

export type NomiCreativeModelSelectProps = Omit<CreativeModelSelectProps, 'catalog'>;

/** Convenience boundary that connects the controlled picker to NomiFun. */
const NomiCreativeModelSelect: React.FC<NomiCreativeModelSelectProps> = (props) => {
  const catalog = useNomiCreativeModelCatalog();
  return <CreativeModelSelect {...props} catalog={catalog} />;
};

export default NomiCreativeModelSelect;
