/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { requestCreativeCanvasProductBeforeLeave } from '../canvas/product/beforeLeave';

/** Keep every app-level navigation surface behind the active product CAS gates. */
export async function requestCreativeStudioBeforeLeave(): Promise<boolean> {
  return requestCreativeCanvasProductBeforeLeave();
}
