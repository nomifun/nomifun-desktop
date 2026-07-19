/**
 * A backend `succeeded` status is only useful when it names at least one
 * persisted artifact. Keep this runtime invariant in one pure helper so the
 * generator card and loop runner cannot drift into different false-success
 * behavior.
 */

import type { AssetId } from '@/common/types/ids';

export const EMPTY_GENERATION_ARTIFACTS_ERROR = 'Generation completed without any persisted artifacts.';

export interface SucceededTaskResult {
  status: string;
  result_asset_ids?: AssetId[] | null;
}

/** Return persisted ids only for a genuine, non-empty successful result. */
export function succeededArtifactIds(task: SucceededTaskResult): AssetId[] | null {
  if (task.status !== 'succeeded') return null;
  const ids = task.result_asset_ids;
  return Array.isArray(ids) && ids.length > 0 ? ids : null;
}
