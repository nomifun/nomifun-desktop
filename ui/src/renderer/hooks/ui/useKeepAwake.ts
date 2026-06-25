/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useState } from 'react';
import { application, systemSettings } from '@/common/adapter/ipcBridge';
import { configService } from '@/common/config/configService';

// 默认开启:未持久化时视为 true / Keep-awake defaults to ON when unset.
const readKeepAwake = (): boolean => configService.get('system.keepAwake') ?? true;

/**
 * 共享的"保持唤醒"状态。toggle 时:乐观更新 -> 应用 OS 效果(applyKeepAwake)-> 持久化(HTTP PUT)。
 * 定时任务页与「设置->系统」都用本 hook,经 configService.subscribe 自动双向同步。
 */
export function useKeepAwake(): { keepAwake: boolean; setKeepAwake: (enabled: boolean) => Promise<void> } {
  const [keepAwake, setKeepAwakeState] = useState<boolean>(readKeepAwake);

  useEffect(() => {
    const unsub = configService.subscribe('system.keepAwake', (v) => setKeepAwakeState(v == null ? true : !!v));
    return unsub;
  }, []);

  const setKeepAwake = useCallback(async (enabled: boolean) => {
    const prev = configService.get('system.keepAwake');
    configService.setLocal('system.keepAwake', enabled); // 乐观 + 通知订阅者
    try {
      await application.applyKeepAwake.invoke({ enabled }); // OS 效果(非桌面无操作)
      await systemSettings.setKeepAwake.invoke({ enabled }); // 持久化到后端
    } catch (err) {
      configService.setLocal('system.keepAwake', prev ?? true); // 回滚
      throw err;
    }
  }, []);

  return { keepAwake, setKeepAwake };
}
