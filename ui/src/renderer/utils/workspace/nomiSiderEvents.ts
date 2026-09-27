/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { createContentSiderChannel } from '@/renderer/components/layout/ContentSider/createContentSiderChannel';

/** Keep the titlebar toggle and the companion roster ContentSider in sync. */
export const nomiSiderChannel = createContentSiderChannel('nomi');
export const NOMI_SIDER_TOGGLE_EVENT = nomiSiderChannel.toggleEvent;
export const dispatchNomiSiderStateEvent = nomiSiderChannel.dispatchState;
