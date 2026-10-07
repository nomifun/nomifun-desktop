/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import type { KnowledgeBaseId } from '@/common/types/ids';

export interface KnowledgeBaseOption {
  value: KnowledgeBaseId;
  label: string;
}

/** Knowledge-base multi-select options (shared knowledge catalog). */
export const useKnowledgeBaseOptions = () => {
  const [options, setOptions] = useState<KnowledgeBaseOption[]>([]);
  const [loading, setLoading] = useState(true);
  const requests = useRef({ active: false, latest: 0 }).current;

  const refresh = useCallback(async () => {
    if (!requests.active) return;
    const request = ++requests.latest;
    const isCurrent = () => requests.active && request === requests.latest;
    setLoading(true);
    try {
      const bases = (await ipcBridge.knowledge.listBases.invoke()) ?? [];
      if (isCurrent()) setOptions(bases.map((base) => ({ value: base.knowledge_base_id, label: base.name })));
    } catch {
      if (isCurrent()) setOptions([]);
    } finally {
      if (isCurrent()) setLoading(false);
    }
  }, [requests]);

  useEffect(() => {
    requests.active = true;
    void refresh();
    return () => { requests.active = false; requests.latest++; };
  }, [refresh, requests]);

  return { options, loading, refresh };
};
