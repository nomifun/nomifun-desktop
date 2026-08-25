/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTask } from '@/common/config/storage';

export type ImageReferenceInputPolicy =
  | { kind: 'none'; maxInputs: 0 }
  | { kind: 'bounded'; maxInputs: number }
  | { kind: 'multiple'; maxInputs: null }
  | { kind: 'unknown'; maxInputs: null };

/** Product safety ceiling, independent from any Provider-advertised limit. */
export const IMAGE_REFERENCE_PRODUCT_MAX_INPUTS = 8;

const NO_REFERENCE_INPUTS: ImageReferenceInputPolicy = {
  kind: 'none',
  maxInputs: 0,
};

const UNKNOWN_REFERENCE_INPUTS: ImageReferenceInputPolicy = {
  kind: 'unknown',
  maxInputs: null,
};

/**
 * Describe image-reference input support from the exact persisted protocol and
 * task. Model names are intentionally ignored: an unknown protocol/task pair
 * remains unknown instead of inheriting a neighbouring provider's behavior.
 *
 * `multiple` means the adapter has an explicit multi-image transport but this
 * product contract does not yet expose a reliable numeric upper bound.
 */
export function imageReferenceInputPolicy(
  protocol: string | null | undefined,
  task: ModelTask
): ImageReferenceInputPolicy {
  if (task === 'image_generation') return NO_REFERENCE_INPUTS;
  if (task !== 'image_edit') return UNKNOWN_REFERENCE_INPUTS;

  switch (protocol) {
    case 'stepfun.images':
      return { kind: 'bounded', maxInputs: 1 };
    case 'ark.images':
      return { kind: 'bounded', maxInputs: IMAGE_REFERENCE_PRODUCT_MAX_INPUTS };
    case 'siliconflow.images':
    case 'xai.images_json':
      return { kind: 'bounded', maxInputs: 3 };
    case 'openai.images':
    case 'gemini.generate_content':
      return { kind: 'multiple', maxInputs: null };
    default:
      return UNKNOWN_REFERENCE_INPUTS;
  }
}

/** Effective Canvas limit without presenting the product ceiling as Provider metadata. */
export function effectiveImageReferenceInputLimit(
  policy: ImageReferenceInputPolicy
): number | null {
  switch (policy.kind) {
    case 'none':
      return 0;
    case 'bounded':
      return Math.min(policy.maxInputs, IMAGE_REFERENCE_PRODUCT_MAX_INPUTS);
    case 'multiple':
      return IMAGE_REFERENCE_PRODUCT_MAX_INPUTS;
    case 'unknown':
      return null;
  }
}
