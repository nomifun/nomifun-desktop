/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { parseAssetId } from '@/common/types/ids';

import { isCreativeAssetDeleted, type CreativeAsset } from '../../assets';
import type { StandaloneWorkbenchDraftKind } from './types';

export interface StandaloneWorkbenchDraftAssetReader {
  get(assetId: string): Promise<CreativeAsset>;
}

export interface HydratedStandaloneWorkbenchDraftReferences {
  assets: CreativeAsset[];
  retainedReferenceAssetIds: string[];
  unavailableReferenceAssetIds: string[];
}

/**
 * Restore only exact assets still readable through the canonical asset API.
 * Missing, malformed, duplicate, mismatched, or non-image references are
 * omitted instead of being represented by stale browser objects.
 */
export async function hydrateStandaloneWorkbenchDraftReferences(
  workbenchKind: StandaloneWorkbenchDraftKind,
  referenceAssetIds: readonly string[],
  assets: StandaloneWorkbenchDraftAssetReader
): Promise<HydratedStandaloneWorkbenchDraftReferences> {
  const maximum = workbenchKind === 'video' ? 1 : 100;
  const seen = new Set<string>();
  const attempts = referenceAssetIds.map(async (candidate, index) => {
    let assetId: string;
    try {
      assetId = parseAssetId(candidate);
    } catch {
      return { index, candidate, asset: null };
    }
    if (index >= maximum || seen.has(assetId)) {
      return { index, candidate: assetId, asset: null };
    }
    seen.add(assetId);
    try {
      const asset = await assets.get(assetId);
      if (asset.id !== assetId || asset.kind !== 'image' || isCreativeAssetDeleted(asset)) {
        return { index, candidate: assetId, asset: null };
      }
      return { index, candidate: assetId, asset };
    } catch {
      return { index, candidate: assetId, asset: null };
    }
  });
  const resolved = (await Promise.all(attempts)).sort((left, right) => left.index - right.index);
  return {
    assets: resolved.flatMap((entry) => (entry.asset ? [entry.asset] : [])),
    retainedReferenceAssetIds: resolved.flatMap((entry) =>
      entry.asset ? [entry.asset.id] : []
    ),
    unavailableReferenceAssetIds: resolved.flatMap((entry) =>
      entry.asset ? [] : [entry.candidate]
    ),
  };
}

/** Exact coordinates only: a same-named model from another Provider is not a substitute. */
export function isExactWorkbenchDraftModelAvailable(
  model: { providerId: string; model: string },
  options: readonly { providerId: string; model: string }[]
): boolean {
  return options.some(
    (option) => option.providerId === model.providerId && option.model === model.model
  );
}
