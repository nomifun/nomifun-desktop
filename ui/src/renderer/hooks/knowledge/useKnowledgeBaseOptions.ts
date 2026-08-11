/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useState } from 'react';
import { ipcBridge } from '@/common';
import type { KnowledgeBaseId } from '@/common/types/ids';

export interface KnowledgeBaseOption {
  value: KnowledgeBaseId;
  label: string;
  /** On-disk root, shown as a secondary line where two bases share a name. */
  rootPath: string;
}

/** Knowledge-base select options (shared knowledge catalog). */
export const useKnowledgeBaseOptions = () => {
  const [options, setOptions] = useState<KnowledgeBaseOption[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const bases = (await ipcBridge.knowledge.listBases.invoke()) ?? [];
      setOptions(
        bases.map((base) => ({
          value: base.knowledge_base_id,
          label: base.name,
          rootPath: base.root_path,
        }))
      );
    } catch {
      setOptions([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { options, loading, refresh };
};
