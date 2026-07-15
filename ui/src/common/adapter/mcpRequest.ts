/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMcpServer, IMcpServerTransport } from '@/common/config/storage';
import type { McpServerId } from '@/common/types/ids';

export interface McpConnectionTestRequest {
  id?: McpServerId;
  name: string;
  transport: IMcpServerTransport;
}

export const buildMcpConnectionTestRequest = (
  server: Pick<IMcpServer, 'id' | 'name' | 'transport'>
): McpConnectionTestRequest => ({
  id: server.id,
  name: server.name,
  transport: server.transport,
});
