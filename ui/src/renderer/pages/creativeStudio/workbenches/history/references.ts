/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CreativeAssetDeletedError, isCreativeAssetDeleted, type CreativeAsset } from '../../assets';
import type { CreativeTask } from '../../tasks';
import type { CreativeWorkbenchReferences } from '../runtime';

export interface CreativeAssetReader {
  get(assetId: string): Promise<CreativeAsset>;
}

export async function hydrateStandaloneTaskReferences(
  task: CreativeTask,
  assets: CreativeAssetReader
): Promise<CreativeWorkbenchReferences> {
  if (task.inputs === null) {
    throw new Error(`Task ${task.taskId} has no proven input snapshot`);
  }
  const resolved = await Promise.all(
    task.inputs.map((binding) => assets.get(binding.assetId))
  );
  task.inputs.forEach((binding, index) => {
    const asset = resolved[index];
    if (asset && isCreativeAssetDeleted(asset)) throw new CreativeAssetDeletedError(asset.id);
    if (!asset || asset.id !== binding.assetId || asset.kind !== binding.kind) {
      throw new Error(
        `Task ${task.taskId} input ${binding.assetId} no longer matches its ${binding.kind} snapshot`
      );
    }
  });
  return {
    bindings: task.inputs.map((binding) => ({ ...binding })),
    assets: resolved,
  };
}
