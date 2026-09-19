import type { ConversationId } from '@/common/types/ids';
import { WORKSPACE_HEADER_HEIGHT } from '@/renderer/pages/conversation/utils/layoutCalc';
import { ExpandLeft, ExpandRight } from '@icon-park/react';
import React from 'react';
import WorkspaceBindButton from './WorkspaceBindButton';
import WorkspaceOpenButton from './WorkspaceOpenButton';
import type { WorkspaceTab } from '@/renderer/pages/conversation/Workspace/types';

type WorkspaceHeaderProps = {
  children?: React.ReactNode;
  showToggle?: boolean;
  collapsed: boolean;
  onToggle: () => void;
  togglePlacement?: 'left' | 'right';
  workspacePath?: string;
  /**
   * Authoritative temp-workspace flag from
   * `conversation.extra.is_temporary_workspace`. Drives which right-side action
   * renders: temp sessions get a new-conversation workspace entry, while bound
   * workspaces get {@link WorkspaceOpenButton}.
   */
  isTemporaryWorkspace?: boolean;
  /**
   * Conversation this panel belongs to. Used only to decide whether the
   * new-conversation workspace entry is available.
   */
  conversation_id?: ConversationId;
  activeTab?: WorkspaceTab;
};

// Compact header bar for the workspace side panel with optional collapse toggle
const WorkspacePanelHeader: React.FC<WorkspaceHeaderProps> = ({
  children,
  showToggle = false,
  collapsed,
  onToggle,
  togglePlacement = 'right',
  workspacePath,
  isTemporaryWorkspace = false,
  conversation_id,
  activeTab = 'files',
}) => (
  <div
    className='workspace-panel-header flex items-center justify-start px-12px py-4px gap-12px border-b border-b-solid border-[var(--bg-3)]'
    style={{ height: WORKSPACE_HEADER_HEIGHT, minHeight: WORKSPACE_HEADER_HEIGHT }}
  >
    {showToggle && togglePlacement === 'left' && (
      <button
        type='button'
        className='workspace-header__toggle mr-4px'
        aria-label='Toggle workspace'
        onClick={onToggle}
      >
        {collapsed ? <ExpandRight size={16} /> : <ExpandLeft size={16} />}
      </button>
    )}
    <div className='workspace-panel-header__title flex-1 truncate'>{children}</div>

    {/* Right-side workspace action. Temporary sessions offer a "new conversation
        in a real directory" entry; bound
        workspaces offer the "open in external tool" button. Each guards for the
        desktop shell internally, so nothing renders in WebUI/browser mode. */}
    {!collapsed &&
      activeTab === 'files' &&
      (isTemporaryWorkspace ? (
        <WorkspaceBindButton conversation_id={conversation_id} />
      ) : (
        workspacePath && <WorkspaceOpenButton workspacePath={workspacePath} isTemporary={false} />
      ))}

    {showToggle && togglePlacement === 'right' && (
      <button type='button' className='workspace-header__toggle' aria-label='Toggle workspace' onClick={onToggle}>
        {collapsed ? <ExpandRight size={16} /> : <ExpandLeft size={16} />}
      </button>
    )}
  </div>
);

export default WorkspacePanelHeader;
