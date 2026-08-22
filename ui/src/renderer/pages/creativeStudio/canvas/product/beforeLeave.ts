/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type CreativeCanvasProductBeforeLeave = () => Promise<boolean>;

interface RegisteredBeforeLeave {
  token: symbol;
  run: CreativeCanvasProductBeforeLeave;
}

let activeBeforeLeave: RegisteredBeforeLeave | null = null;

/**
 * Register the currently mounted product route's CAS gate. The shell owns
 * navigation; this module only supplies the active route decision.
 */
export function registerCreativeCanvasProductBeforeLeave(
  run: CreativeCanvasProductBeforeLeave
): () => void {
  const registration = { token: Symbol('creative-canvas-before-leave'), run };
  activeBeforeLeave = registration;
  return () => {
    if (activeBeforeLeave?.token === registration.token) activeBeforeLeave = null;
  };
}

/** App-level navigation should await this and continue only on `true`. */
export async function requestCreativeCanvasProductBeforeLeave(): Promise<boolean> {
  const registration = activeBeforeLeave;
  if (!registration) return true;
  try {
    return await registration.run();
  } catch {
    return false;
  }
}

export function hasCreativeCanvasProductBeforeLeave(): boolean {
  return activeBeforeLeave !== null;
}
