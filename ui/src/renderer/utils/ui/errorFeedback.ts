/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isBackendHttpError, redactSensitiveText } from '@/common/adapter/httpBridge';

/** Safe diagnostics for error feedback and its copy action. */
export function getErrorTechnicalDetails(error: unknown): string {
  if (isBackendHttpError(error)) {
    // httpBridge has already redacted credentials in its structured body and
    // message. Preserve that diagnostic instead of showing it as user copy.
    return redactSensitiveText(`${error.name}: ${error.message}`);
  }
  if (error instanceof Error) return redactSensitiveText(`${error.name}: ${error.message}`);
  if (typeof error === 'string') return redactSensitiveText(error);
  // Arbitrary thrown objects may contain credentials. Do not dump them into
  // UI or clipboard outside the bridge's redacted response contract.
  return 'Unknown error';
}
