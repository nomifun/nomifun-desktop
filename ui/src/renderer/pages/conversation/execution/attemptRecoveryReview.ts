type AttemptRecoveryReview = { kind?: string; reason?: string; operationId?: string; receiptState?: string };

/** The repository's explicit review barrier, never inferred from question prose. */
export function attemptRecoveryReview(runtimeState: unknown): AttemptRecoveryReview | null {
  if (!runtimeState || typeof runtimeState !== 'object' || !('review_blocked' in runtimeState) || runtimeState.review_blocked == null) return null;
  const review = runtimeState.review_blocked;
  // Unknown future review markers also suppress an answer form that cannot resume.
  if (typeof review !== 'object') return {};
  return {
    kind: 'kind' in review && typeof review.kind === 'string' ? review.kind : undefined,
    reason: 'reason' in review && typeof review.reason === 'string' ? review.reason : undefined,
    operationId: 'operation_id' in review && typeof review.operation_id === 'string' ? review.operation_id : undefined,
    receiptState: 'receipt_state' in review && typeof review.receipt_state === 'string' ? review.receipt_state : undefined,
  };
}
