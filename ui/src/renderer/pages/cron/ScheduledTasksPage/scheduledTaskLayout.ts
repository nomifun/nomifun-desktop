/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type ScheduledTaskLayout = 'card' | 'row';

export const DESKTOP_SCHEDULED_TASK_COLUMNS =
  'minmax(0,1.6fr) minmax(150px,1.1fr) minmax(84px,auto) minmax(120px,1fr) 44px';

export function getScheduledTaskLayout(isMobile: boolean): ScheduledTaskLayout {
  return isMobile ? 'card' : 'row';
}
