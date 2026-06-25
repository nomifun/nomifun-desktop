/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { useEffect, useMemo, useState } from 'react';

function gitMetadataPath(workpath: string): string {
  const trimmed = workpath.replace(/[\\/]+$/, '');
  const separator = trimmed.includes('\\') && !trimmed.includes('/') ? '\\' : '/';
  return `${trimmed}${separator}.git`;
}

export function useWorkpathBranches(workpaths: string[], enabled: boolean): Map<string, string> {
  const [branches, setBranches] = useState<Map<string, string>>(() => new Map());
  const workpathKey = useMemo(() => Array.from(new Set(workpaths.filter(Boolean))).sort().join('\u0000'), [workpaths]);
  const stableWorkpaths = useMemo(() => (workpathKey ? workpathKey.split('\u0000') : []), [workpathKey]);

  useEffect(() => {
    if (!enabled || stableWorkpaths.length === 0) {
      setBranches(new Map());
      return undefined;
    }

    let cancelled = false;

    const load = async () => {
      const entries = await Promise.all(
        stableWorkpaths.map(async (workpath): Promise<[string, string | null]> => {
          try {
            await ipcBridge.fs.getFileMetadata.invoke({ path: gitMetadataPath(workpath), workspace: workpath });
          } catch {
            return [workpath, null];
          }

          try {
            const info = await ipcBridge.fileSnapshot.init.invoke({ workspace: workpath });
            if (info.mode !== 'disabled') {
              void ipcBridge.fileSnapshot.dispose.invoke({ workspace: workpath }).catch(() => {});
            }
            return [workpath, info.mode === 'git-repo' && info.branch ? info.branch : null];
          } catch {
            return [workpath, null];
          }
        })
      );

      if (cancelled) return;
      setBranches(new Map(entries.filter((entry): entry is [string, string] => !!entry[1])));
    };

    void load();
    return () => {
      cancelled = true;
    };
  }, [enabled, stableWorkpaths, workpathKey]);

  return branches;
}
