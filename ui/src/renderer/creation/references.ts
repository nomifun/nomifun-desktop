import { parseAssetId } from '@/common/types/ids';
import { CreativeAssetDeletedError, isCreativeAssetDeleted, type CreativeAsset, type CreativeAssetKind } from '@renderer/pages/creativeStudio/assets';
import type { CreativeTaskInputRole } from '@renderer/pages/creativeStudio/tasks';
import { GenerationError } from './generationError';

interface GenerationReferenceBinding {
  assetId: string;
  kind: CreativeAssetKind;
  role: CreativeTaskInputRole;
}

/**
 * Reference assets must be concrete objects selected from the existing asset
 * port. Asset origin is provenance, not an ownership proof.
 */
export interface GenerationReferences {
  bindings: readonly GenerationReferenceBinding[];
  assets: readonly CreativeAsset[];
}

/**
 * Validate references against concrete objects selected from the current-user
 * asset port. We deliberately do not treat optional origin fields as ACLs.
 */
export function validateGenerationReferences(
  references: GenerationReferences,
): CreativeAsset[] {
  const suppliedIds = references.assets.map((asset) => asset.id);
  if (
    new Set(suppliedIds).size !== suppliedIds.length ||
    references.assets.length !== references.bindings.length
  ) {
    throw new GenerationError(
      "reference_contract_mismatch",
      "Reference assets must match bindings one-to-one without duplicates",
      "assets",
    );
  }
  const assets = new Map(references.assets.map((asset) => [asset.id, asset]));
  const seen = new Set<string>();
  return references.bindings.map((binding, index) => {
    const assetId = String(parseAssetId(binding.assetId));
    if (seen.has(assetId)) {
      throw new GenerationError(
        "reference_contract_mismatch",
        `Reference asset ${assetId} is duplicated`,
        `bindings[${index}].assetId`,
      );
    }
    seen.add(assetId);
    const asset = assets.get(assetId);
    if (asset && isCreativeAssetDeleted(asset)) throw new CreativeAssetDeletedError(asset.id);
    if (!asset) {
      throw new GenerationError(
        "reference_not_owned",
        `Reference asset ${assetId} was not supplied by the asset selection boundary`,
        `bindings[${index}].assetId`,
      );
    }
    if (asset.kind !== binding.kind) {
      throw new GenerationError(
        "reference_kind_mismatch",
        `Reference asset ${assetId} is ${asset.kind}, not ${binding.kind}`,
        `bindings[${index}].kind`,
      );
    }
    return asset;
  });
}
