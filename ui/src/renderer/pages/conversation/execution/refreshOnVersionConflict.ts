import { isBackendHttpError } from '@/common/adapter/httpBridge';

/** Refresh canonical execution state after an optimistic-concurrency conflict. */
export async function refreshOnVersionConflict(error: unknown, refetch: () => Promise<void>): Promise<boolean> {
  if (!isBackendHttpError(error) || error.status !== 409) return false;
  try {
    await refetch();
  } catch {
    // Preserve the original mutation error; the live event stream can retry the refresh.
  }
  return true;
}
