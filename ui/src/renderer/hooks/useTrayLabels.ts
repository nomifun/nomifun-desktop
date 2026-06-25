/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import { isDesktopShell } from '@/renderer/utils/platform';

/**
 * 把原生系统托盘菜单的标签(「显示 NomiFun」「退出」)同步为当前 UI 语言。
 *
 * Rust 侧(apps/desktop/src/main.rs)创建托盘时只能用英文兜底——它无法解析前端 i18n。
 * 本 hook 在挂载时及语言切换时,把译文经 `set_tray_labels` 命令推给托盘菜单项。
 * 仅在桌面壳生效;WebUI 浏览器下 `setTrayLabels` 为 no-op。
 *
 * Sync the native system-tray menu labels (Show / Quit) to the current UI locale.
 * No-op outside the Tauri desktop shell.
 */
export function useTrayLabels(): void {
  const { t, i18n } = useTranslation();
  useEffect(() => {
    if (!isDesktopShell()) return;
    void ipcBridge.application.setTrayLabels
      .invoke({ show: t('common.tray.showWindow'), quit: t('common.tray.quit') })
      .catch(() => {});
  }, [t, i18n.language]);
}
