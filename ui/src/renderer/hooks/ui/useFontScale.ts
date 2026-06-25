/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import { useCallback, useEffect, useState } from 'react';

const UI_SCALE_DEFAULT = 1;
const UI_SCALE_MIN = 0.8;
const UI_SCALE_MAX = 1.3;
const UI_SCALE_STEP = 0.05;

export const FONT_SCALE_DEFAULT = UI_SCALE_DEFAULT;
export const FONT_SCALE_MIN = UI_SCALE_MIN;
export const FONT_SCALE_MAX = UI_SCALE_MAX;
export const FONT_SCALE_STEP = UI_SCALE_STEP;

// 确保缩放值在允许范围内 / Clamp UI scale to allowed range
const clampFontScale = (value: number) => {
  if (Number.isNaN(value) || !Number.isFinite(value)) {
    return FONT_SCALE_DEFAULT;
  }
  return Math.min(FONT_SCALE_MAX, Math.max(FONT_SCALE_MIN, value));
};

const useFontScale = (): [number, (scale: number) => Promise<void>] => {
  const [fontScale, setFontScaleState] = useState(FONT_SCALE_DEFAULT);

  // 启动时从持久化配置(ui.zoomFactor)恢复缩放，并实际应用到 webview。
  // Tauri 的 webview zoom 每次启动重置为 1，仅恢复滑块状态不够，必须主动 setZoom。
  // Restore persisted zoom on launch and re-apply it to the webview (Tauri resets
  // webview zoom to 1 each launch, so updating slider state alone is not enough).
  const restoreZoomFactor = useCallback(async () => {
    try {
      await configService.whenReady();
      const stored = configService.get('ui.zoomFactor');
      const factor = typeof stored === 'number' ? clampFontScale(stored) : FONT_SCALE_DEFAULT;
      setFontScaleState(factor);
      await ipcBridge.application.setZoomFactor.invoke({ factor });
    } catch (error) {
      console.error('Failed to restore zoom factor:', error);
    }
  }, []);

  useEffect(() => {
    void restoreZoomFactor();
  }, [restoreZoomFactor]);

  // 乐观更新 slider，应用 zoom，并持久化到后端配置(重启后可恢复)。
  // Optimistically update slider, apply zoom, and persist so it survives restart.
  const setFontScale = useCallback(
    async (nextScale: number) => {
      const clamped = clampFontScale(nextScale);
      setFontScaleState(clamped);
      try {
        const updatedFactor = await ipcBridge.application.setZoomFactor.invoke({ factor: clamped });
        const applied = typeof updatedFactor === 'number' ? clampFontScale(updatedFactor) : clamped;
        if (applied !== clamped) setFontScaleState(applied);
        await configService.set('ui.zoomFactor', applied);
      } catch (error) {
        console.error('Failed to set zoom factor:', error);
        void restoreZoomFactor();
      }
    },
    [restoreZoomFactor]
  );

  return [fontScale, setFontScale];
};

export { clampFontScale };
export default useFontScale;
