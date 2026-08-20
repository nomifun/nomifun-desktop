/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { toPromptLibrarySelection } from '../library';
import type { PromptLibraryItem, PromptLibrarySelection } from '../types';

export type PromptClipboardWriter = (text: string) => Promise<void>;

/**
 * Standalone prompt-library routes have no canvas insertion target. Copying is
 * therefore the only product action: it writes the validated prompt verbatim
 * and returns the same immutable selection shape used by contextual editors.
 */
export async function copyStandalonePrompt(
  item: PromptLibraryItem,
  writeText: PromptClipboardWriter
): Promise<PromptLibrarySelection> {
  await writeText(item.prompt);
  return toPromptLibrarySelection(item);
}
