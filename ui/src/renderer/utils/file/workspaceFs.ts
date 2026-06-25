/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { ipcBridge } from '@/common';

export const removeWorkspaceEntry = (path: string) => {
  return ipcBridge.fs.removeEntry.invoke({ path });
};

export const renameWorkspaceEntry = (path: string, new_name: string) => {
  return ipcBridge.fs.renameEntry.invoke({ path, new_name });
};
