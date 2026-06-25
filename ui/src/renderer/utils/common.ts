/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

export const removeStack = (...args: Array<() => void>) => {
  return () => {
    const list = args.slice();
    while (list.length) {
      list.pop()!();
    }
  };
};

/**
 * Tool confirmation outcome enum
 * Standalone copy of the tool-confirmation outcome enum, kept local to avoid
 * pulling Node.js-only dependencies (node:crypto) into the renderer process.
 */
export enum ToolConfirmationOutcome {
  ProceedOnce = 'proceed_once',
  ProceedAlways = 'proceed_always',
  ProceedAlwaysServer = 'proceed_always_server',
  ProceedAlwaysTool = 'proceed_always_tool',
  ModifyWithEditor = 'modify_with_editor',
  Cancel = 'cancel',
}

export { uuid } from '@/common/utils';
