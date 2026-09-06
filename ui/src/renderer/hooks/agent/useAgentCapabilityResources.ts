/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CapabilityCatalogItem,
  ExactCatalogRef,
} from '@/common/types/agentPlatform';

const referenceKey = (
  reference: Pick<ExactCatalogRef<'capability'>, 'id' | 'version'>
): string => `${reference.id}@${reference.version}`;

export const requiredResourceKindsForCapabilityReferences = (
  references: readonly Pick<ExactCatalogRef<'capability'>, 'id' | 'version'>[],
  catalog: readonly CapabilityCatalogItem[]
): Set<string> => {
  const selected = new Set(references.map(referenceKey));
  return new Set(
    catalog
      .filter((item) => selected.has(referenceKey(item.capability)))
      .flatMap((item) => item.required_resource_kinds)
  );
};
