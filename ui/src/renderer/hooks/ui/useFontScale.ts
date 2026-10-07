/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import { useCallback, useEffect, useRef, useState } from 'react';

export const FONT_SCALE_DEFAULT = 1;
export const FONT_SCALE_MIN = 0.8;
export const FONT_SCALE_MAX = 1.3;
export const FONT_SCALE_STEP = 0.05;

// 确保缩放值在允许范围内 / Clamp UI scale to allowed range
const clampFontScale = (value: number) => {
  if (!Number.isFinite(value)) {
    return FONT_SCALE_DEFAULT;
  }
  return Math.min(FONT_SCALE_MAX, Math.max(FONT_SCALE_MIN, value));
};

const readFontScale = (config: typeof configService): number => {
  const stored = config.get('ui.zoomFactor');
  return typeof stored === 'number' ? clampFontScale(stored) : FONT_SCALE_DEFAULT;
};

const useFontScale = (
  config = configService,
  setZoom = ipcBridge.application.setZoomFactor.invoke
): [number, (scale: number) => Promise<void>] => {
  const [fontScale, setFontScaleState] = useState(() => readFontScale(config));
  const revision = useRef(0);
  const saving = useRef(false);
  const zoomQueue = useRef<Promise<unknown>>(Promise.resolve());

  // Native setZoom cannot be cancelled once dispatched. Keep calls ordered in
  // this hook; skip queued obsolete targets and never persist their results.
  const applyZoom = useCallback((factor: number, current: number) => {
    const next = zoomQueue.current.then(async () => {
      if (revision.current !== current) return undefined;
      const result = await setZoom({ factor });
      return typeof result === 'number' ? clampFontScale(result) : factor;
    });
    zoomQueue.current = next.catch(() => {});
    return next;
  }, [setZoom]);

  // 启动时从持久化配置(ui.zoomFactor)恢复缩放，并实际应用到 webview。
  // Tauri 的 webview zoom 每次启动重置为 1，仅恢复滑块状态不够，必须主动 setZoom。
  // Restore persisted zoom on launch and re-apply it to the webview (Tauri resets
  // webview zoom to 1 each launch, so updating slider state alone is not enough).
  const restoreZoomFactor = useCallback(() => {
    // A reload during a user operation must not reset the slider/native zoom
    // before that operation has either persisted or failed.
    if (saving.current) return;
    const current = ++revision.current;
    const factor = readFontScale(config);
    setFontScaleState(factor);
    void applyZoom(factor, current).then((applied) => {
      if (revision.current === current && applied !== undefined) setFontScaleState(applied);
    }).catch((error: unknown) => {
      if (revision.current === current) console.error('Failed to restore zoom factor:', error);
    });
  }, [config, applyZoom]);

  useEffect(() => {
    let active = true;
    let notified = false;
    const unsubscribe = config.subscribe('ui.zoomFactor', () => {
      notified = true;
      restoreZoomFactor();
    });
    void config.whenReady().then(() => {
      if (active && !notified) restoreZoomFactor();
    });
    return () => {
      active = false;
      saving.current = false;
      revision.current++;
      unsubscribe();
    };
  }, [config, restoreZoomFactor]);

  // 乐观更新 slider，应用 zoom，并持久化到后端配置(重启后可恢复)。
  // Optimistically update slider, apply zoom, and persist so it survives restart.
  const setFontScale = useCallback(
    async (nextScale: number) => {
      const previous = readFontScale(config);
      const clamped = clampFontScale(nextScale);
      const current = ++revision.current;
      saving.current = true;
      setFontScaleState(clamped);
      let writeStarted = false;
      let applied: number | undefined;
      try {
        applied = await applyZoom(clamped, current);
        if (revision.current !== current || applied === undefined) return;
        setFontScaleState(applied);
        writeStarted = true;
        await config.set('ui.zoomFactor', applied);
        if (revision.current === current) {
          saving.current = false;
          if (readFontScale(config) !== applied) restoreZoomFactor();
        }
      } catch (error) {
        console.error('Failed to set zoom factor:', error);
        if (revision.current === current) {
          saving.current = false;
          if (writeStarted && config.get('ui.zoomFactor') === applied) {
            config.setLocal('ui.zoomFactor', previous);
          } else {
            restoreZoomFactor();
          }
        }
        // Native failures never changed preferences. Failed PUTs do need an
        // authoritative reload, including when a newer action superseded them.
        if (writeStarted) await config.reload();
      }
    },
    [restoreZoomFactor, config, applyZoom]
  );

  return [fontScale, setFontScale];
};

export { clampFontScale };
export default useFontScale;
