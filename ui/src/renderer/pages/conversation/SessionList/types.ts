/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import type { AutoWorkRunState, IdmmRunState } from '@/common/adapter/ipcBridge';
import type { TChatConversation } from '@/common/config/storage';

export type ExportZipFile = {
  name: string;
  content?: string;
  sourcePath?: string;
};

export type ExportTask =
  | { mode: 'single'; conversation: TChatConversation }
  | { mode: 'batch'; conversation_ids: number[] }
  | null;

export type ConversationRowProps = {
  conversation: TChatConversation;
  isGenerating: boolean;
  hasCompletionUnread: boolean;
  collapsed: boolean;
  tooltipEnabled: boolean;
  batchMode: boolean;
  checked: boolean;
  selected: boolean;
  menuVisible: boolean;
  onToggleChecked: (conversation: TChatConversation) => void;
  onConversationClick: (conversation: TChatConversation) => void;
  onOpenMenu: (conversation: TChatConversation) => void;
  onMenuVisibleChange: (conversation_id: number, visible: boolean) => void;
  onEditStart: (conversation: TChatConversation) => void;
  onDelete: (conversation_id: number) => void;
  onExport?: (conversation: TChatConversation) => void;
  onTogglePin: (conversation: TChatConversation) => void;
  getJobStatus: (conversation_id: number) => 'none' | 'active' | 'paused' | 'error' | 'unread';
  /** AutoWork run state when enabled for this conversation (undefined = not enabled / unknown). */
  autoworkState?: AutoWorkRunState;
  /** IDMM run state when enabled for this conversation (undefined = not enabled / unknown). */
  idmmState?: IdmmRunState;
  /** When true, the agent icon is dimmed by default and only shows full color on hover. Used inside project folders to reduce visual weight. */
  dimIcon?: boolean;
  /** Sidebar display preference: show/hide the compact age marker on the right. */
  showSessionAge?: boolean;
};
