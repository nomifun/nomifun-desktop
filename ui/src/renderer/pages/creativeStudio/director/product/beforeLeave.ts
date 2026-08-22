/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type CreativeDirectorProductBeforeLeave = () => Promise<boolean>;

interface RegisteredBeforeLeave {
  token: symbol;
  run: CreativeDirectorProductBeforeLeave;
}

let activeBeforeLeave: RegisteredBeforeLeave | null = null;

export function registerCreativeDirectorProductBeforeLeave(
  run: CreativeDirectorProductBeforeLeave,
): () => void {
  const registration = { token: Symbol("creative-director-before-leave"), run };
  activeBeforeLeave = registration;
  return () => {
    if (activeBeforeLeave?.token === registration.token)
      activeBeforeLeave = null;
  };
}

/** App-level navigation must await this CAS gate and continue only on true. */
export async function requestCreativeDirectorProductBeforeLeave(): Promise<boolean> {
  const registration = activeBeforeLeave;
  if (!registration) return true;
  try {
    return await registration.run();
  } catch {
    return false;
  }
}

export function hasCreativeDirectorProductBeforeLeave(): boolean {
  return activeBeforeLeave !== null;
}
