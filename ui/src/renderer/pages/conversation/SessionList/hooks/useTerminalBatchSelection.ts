/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { TerminalId } from '@/common/types/ids';
import { useCallback, useEffect, useState } from 'react';

export const useTerminalBatchSelection = (batchMode: boolean) => {
  const [selectedTerminalIds, setSelectedTerminalIds] = useState(new Set<TerminalId>());
  useEffect(() => {
    if (!batchMode) setSelectedTerminalIds(new Set());
  }, [batchMode]);
  // A failed list refresh cannot prove removal. Keep failed deletion targets
  // until a successful HTTP receipt or the terminal owner's removed event.
  useEffect(() => ipcBridge.terminal.onRemoved.on(({ terminal_id }) => {
    setSelectedTerminalIds((previous) => {
      if (!previous.has(terminal_id)) return previous;
      const next = new Set(previous);
      next.delete(terminal_id);
      return next;
    });
  }), []);
  const toggleSelectedTerminal = useCallback((id: TerminalId) => {
    setSelectedTerminalIds((previous) => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);
  return { selectedTerminalIds, setSelectedTerminalIds, toggleSelectedTerminal };
};
