import type { ConversationId } from '@/common/types/ids';
import { AgentLogoIcon } from '@/renderer/components/agent/AgentBadge';
import { conversationTarget } from '@/common/types/ids';
import { browserStorageKey } from '@/common/utils/browserStorageKey';
import type { AgentInfo } from '@/renderer/hooks/agent/useAgentInfo';
import FlexFullContainer from '@/renderer/components/layout/FlexFullContainer';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import { useResizableSplit } from '@/renderer/hooks/ui/useResizableSplit';
import ChatTitleEditor from '@/renderer/pages/conversation/components/ChatTitleEditor';
import AutoWorkControl from '@/renderer/pages/conversation/components/AutoWorkControl';
import IdmmControl from '@/renderer/pages/conversation/components/IdmmControl';
import KnowledgeControl from '@/renderer/pages/conversation/components/KnowledgeControl';
import WorkspacePanelHeader from './WorkspacePanelHeader';
import WorkspaceToolRail, {
  WORKSPACE_PANEL_META_EVENT,
  dispatchWorkspacePanelTabEvent,
  type WorkspacePanelMetaDetail,
  type WorkspaceToolRailCollaboration,
} from './WorkspaceToolRail';
import { useContainerWidth } from '@/renderer/hooks/ui/useContainerWidth';
import { useLayoutConstraints } from '@/renderer/pages/conversation/hooks/useLayoutConstraints';
import { usePreviewAutoCollapse } from '@/renderer/pages/conversation/hooks/usePreviewAutoCollapse';
import { useTitleRename } from '@/renderer/pages/conversation/hooks/useTitleRename';
import { useWorkspaceCollapse } from '@/renderer/pages/conversation/hooks/useWorkspaceCollapse';
import { useWorkspacePanelTabs } from '@/renderer/pages/conversation/hooks/useWorkspacePanelTabs';
import { PreviewPanel, PreviewProvider, usePreviewContext } from '@/renderer/pages/conversation/Preview';
import { dispatchWorkspaceToggleEvent } from '@/renderer/utils/workspace/workspaceEvents';
import { useConversationAgents } from '@/renderer/pages/conversation/hooks/useConversationAgents';
import classNames from 'classnames';
import { isDesktopShell, isMacOS, isWindows } from '@/renderer/utils/platform';
import {
  DEFAULT_WORKSPACE_PANEL_PX,
  MAX_WORKSPACE_PANEL_PX,
  MIN_WORKSPACE_PANEL_PX,
  WORKSPACE_HEADER_HEIGHT,
  calcLayoutMetrics,
} from '@/renderer/pages/conversation/utils/layoutCalc';
import { Layout as ArcoLayout } from '@arco-design/web-react';
import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { uuid } from '@/renderer/utils/common';
import type { WorkspaceExtraTab, WorkspaceTab } from '@/renderer/pages/conversation/Workspace/types';
import './chat-layout.css';

// headerExtra allows injecting custom actions (e.g., model picker) into the header's right area
export interface ChatLayoutProps {
  children?: React.ReactNode;
  title?: React.ReactNode;
  sider: React.ReactNode;
  siderTitle?: React.ReactNode;
  backend?: string;
  /** Preset info — when provided, the badge shows the preset identity instead of the backend. */
  preset?: AgentInfo;
  /** Fallback agent name (used when no preset, e.g. from conversation.extra.agent_name) */
  agent_name?: string;
  headerExtra?: React.ReactNode;
  /**
   * Hide the session-capability controls baked into the header
   * (AutoWork / IDMM / Knowledge).
   * Used by surfaces that deliberately offer a reduced feature set — e.g. the
   * desktop companion chat tab. Defaults to false (full conversation page).
   */
  hideAdvancedControls?: boolean;
  /** Whether this Agent's immutable capability ceiling permits target-scoped
   * knowledge binding. Plain Nomi defaults to enabled. */
  knowledgeEnabled?: boolean;
  /**
   * Make the header title read-only (no click-to-rename). Used by single-session
   * surfaces like the companion chat, where the title tracks an external source
   * (the companion name) and a per-conversation rename would desync it.
   */
  disableRename?: boolean;
  /**
   * 嵌套面板（如伙伴聊天 Tab）自带工作区开关：不依赖按路由门控的 app 标题栏。
   * 为 true 时，面板内折叠键与折叠态悬浮展开键无视桌面运行时一律渲染。默认 false，
   * 既有会话/终端表面行为不变（仍由标题栏驱动）。
   */
  selfContainedWorkspaceToggle?: boolean;
  workspaceEnabled?: boolean;
  /** Conversation ID for mode switching */
  conversation_id?: ConversationId;
  /** Custom tabs slot; when provided, replaces the default ConversationTabs */
  tabsSlot?: React.ReactNode;
  /** Workspace path for opening in external tools */
  workspacePath?: string;
  /** Authoritative temp-workspace flag from `conversation.extra.is_temporary_workspace`. */
  isTemporaryWorkspace?: boolean;
  /** Custom rename handler; when provided, replaces the default conversation.update rename flow */
  onRenameTitle?: (new_name: string) => Promise<boolean>;
  /** Optional override for the leading icon shown before the title. */
  headerLeading?: React.ReactNode;
  /** Extra panels exposed by the persistent vertical tool rail. */
  workspaceExtraTabs?: WorkspaceExtraTab[];
  /** Optional collaboration progress entry merged into the same tool rail. */
  workspaceCollaboration?: WorkspaceToolRailCollaboration;
}

/**
 * ChatLayoutInner — the actual chat surface layout. Lives strictly INSIDE the
 * per-surface {@link PreviewProvider} mounted by the {@link ChatLayout} wrapper,
 * so every `usePreviewContext()` consumer in this subtree (including this
 * component's own `isPreviewOpen` read, the `PreviewPanel`, the SendBoxes, the
 * workspace rail, MermaidBlock, …) resolves against THIS surface's provider.
 */
const ChatLayoutInner: React.FC<ChatLayoutProps> = (props) => {
  const { t } = useTranslation();
  const { conversation_id, workspacePath, isTemporaryWorkspace } = props;
  const workspaceTarget = conversation_id != null ? conversationTarget(conversation_id) : undefined;
  const { backend, preset, agent_name, workspaceEnabled = true } = props;
  const layout = useLayoutContext();
  // Native macOS/Windows shells can omit the redundant in-panel toggle because
  // the persistent far-right tool rail remains available. A desktop WebUI must
  // not infer native chrome from the browser user agent.
  const isDesktopRuntime = isDesktopShell();
  const isMacRuntime = isDesktopRuntime && isMacOS();
  const isWindowsRuntime = isDesktopRuntime && isWindows();
  // Preview panel state
  const { isOpen: isPreviewOpen } = usePreviewContext();

  // --- Hook A: workspace collapse ---
  const { rightSiderCollapsed, setRightSiderCollapsed, persistRightSiderCollapsed } = useWorkspaceCollapse({
    workspaceEnabled,
    conversation_id,
    target: workspaceTarget,
    isTemporaryWorkspace,
    autoExpandOnFiles: false,
  });
  const { activeWorkspaceTab, setActiveWorkspaceTab } = useWorkspacePanelTabs(workspaceTarget);

  const activeWorkspaceTitle =
    activeWorkspaceTab === 'files'
      ? props.siderTitle
      : activeWorkspaceTab === 'changes'
        ? t('conversation.workspace.changes.tab')
        : props.workspaceExtraTabs?.find((tab) => tab.key === activeWorkspaceTab)?.title ?? props.siderTitle;

  const selectWorkspaceTool = (tab: string) => {
    const nextTab = tab as WorkspaceTab;
    const clickingActivePanel = !rightSiderCollapsed && activeWorkspaceTab === nextTab;
    if (props.workspaceCollaboration?.active) props.workspaceCollaboration.onClick();
    setActiveWorkspaceTab(nextTab);
    if (workspaceTarget) dispatchWorkspacePanelTabEvent(nextTab, workspaceTarget);
    persistRightSiderCollapsed(clickingActivePanel);
  };

  const workspaceCollaboration = props.workspaceCollaboration
    ? {
        ...props.workspaceCollaboration,
        onClick: () => {
          if (!props.workspaceCollaboration?.active) {
            setRightSiderCollapsed(true);
          }
          props.workspaceCollaboration?.onClick();
        },
      }
    : undefined;

  // --- Hook B: container width ---
  const { ref: containerRef, width: containerWidth } = useContainerWidth<HTMLDivElement>({
    fallbackToWindowWidth: true,
  });

  // --- Hook C: title rename ---
  const { editingTitle, setEditingTitle, titleDraft, setTitleDraft, renameLoading, canRenameTitle, submitTitleRename } =
    useTitleRename({
      title: props.title,
      conversation_id,
      onRename: props.onRenameTitle,
    });

  // Resolve backend display name from detected agents catalog (backend-authoritative).
  // Custom agents live in the same catalog with `agent_source === 'custom'`,
  // so no separate ConfigStorage fallback list is needed.
  const { cliAgents } = useConversationAgents();
  const backendAgentName = backend
    ? cliAgents.find((a) => a.backend === backend || a.agent_type === backend)?.name
    : undefined;
  const capitalizedBackend = backend ? backend.charAt(0).toUpperCase() + backend.slice(1) : backend;

  // Compute display name with fallback chain
  const display_name = preset?.name || agent_name || backendAgentName || capitalizedBackend;

  const {
    splitRatio: workspaceWidthPxPref,
    setSplitRatio: setWorkspaceWidthPxPref,
    createDragHandle: createWorkspaceDragHandle,
  } = useResizableSplit({
    unit: 'px',
    defaultWidth: DEFAULT_WORKSPACE_PANEL_PX,
    minWidth: MIN_WORKSPACE_PANEL_PX,
    maxWidth: MAX_WORKSPACE_PANEL_PX,
    storageKey: 'chat-workspace-width-px',
  });

  // Pre-hook metrics: compute dynamic min/max for the chat-preview split hook
  const { dynamicChatMinRatio, dynamicChatMaxRatio } = calcLayoutMetrics({
    containerWidth,
    workspaceWidthPx: workspaceWidthPxPref,
    chatSplitRatio: 60, // placeholder; only dynamicChatMinRatio/dynamicChatMaxRatio are used here
    workspaceEnabled,
    isPreviewOpen,
    rightSiderCollapsed,
  });

  const {
    splitRatio: chatSplitRatio,
    setSplitRatio: setChatSplitRatio,
    createDragHandle: createPreviewDragHandle,
  } = useResizableSplit({
    defaultWidth: 60,
    minWidth: dynamicChatMinRatio,
    maxWidth: dynamicChatMaxRatio,
    storageKey: 'chat-preview-split-ratio',
  });

  // Full metrics with real chatSplitRatio
  const { chatFlex, workspaceWidthPx, titleAreaMaxWidth } = calcLayoutMetrics({
    containerWidth,
    workspaceWidthPx: workspaceWidthPxPref,
    chatSplitRatio,
    workspaceEnabled,
    isPreviewOpen,
    rightSiderCollapsed,
  });

  // --- Hook D: preview auto-collapse ---
  usePreviewAutoCollapse({
    isPreviewOpen,
    workspaceEnabled,
    rightSiderCollapsed,
    setRightSiderCollapsed,
    siderCollapsed: layout?.siderCollapsed,
    setSiderCollapsed: layout?.setSiderCollapsed,
  });

  // --- Hook E: layout constraints ---
  useLayoutConstraints({
    containerWidth,
    workspaceEnabled,
    isPreviewOpen,
    rightSiderCollapsed,
    setRightSiderCollapsed,
    workspaceWidthPx: workspaceWidthPxPref,
    setWorkspaceWidthPx: setWorkspaceWidthPxPref,
    chatSplitRatio,
    setChatSplitRatio,
    dynamicChatMinRatio,
    dynamicChatMaxRatio,
  });

  const [workspaceChangeCount, setWorkspaceChangeCount] = useState(0);
  useEffect(() => {
    if (typeof window === 'undefined') return undefined;
    const handleWorkspaceMeta = (event: Event) => {
      const detail = (event as CustomEvent<WorkspacePanelMetaDetail>).detail;
      if (
        !detail ||
        !workspaceTarget ||
        detail.target.kind !== workspaceTarget.kind ||
        detail.target.id !== workspaceTarget.id
      ) {
        return;
      }
      setWorkspaceChangeCount(detail.changeCount);
    };
    window.addEventListener(WORKSPACE_PANEL_META_EVENT, handleWorkspaceMeta);
    return () => window.removeEventListener(WORKSPACE_PANEL_META_EVENT, handleWorkspaceMeta);
  }, [workspaceTarget?.id, workspaceTarget?.kind]);

  const desktopHeader = (
    <ArcoLayout.Header
      className={classNames(
        'min-h-44px flex items-center justify-between px-16px pt-8px pb-10px gap-16px !bg-1 chat-layout-header chat-layout-header--glass overflow-hidden'
      )}
    >
      <FlexFullContainer className='h-full min-w-0' containerClassName='flex items-center'>
        <ChatTitleEditor
          editingTitle={editingTitle}
          titleDraft={titleDraft}
          setTitleDraft={setTitleDraft}
          setEditingTitle={setEditingTitle}
          renameLoading={renameLoading}
          canRenameTitle={canRenameTitle && !props.disableRename}
          submitTitleRename={submitTitleRename}
          titleAreaMaxWidth={titleAreaMaxWidth}
          title={props.title}
          conversation_id={conversation_id}
          leading={
            props.headerLeading ??
            ((backend || preset) && (
              <AgentLogoIcon
                backend={backend}
                agent_name={display_name}
                agentLogo={preset?.logo}
                agentLogoIsEmoji={preset?.isEmoji}
              />
            ))
          }
        />
      </FlexFullContainer>
      <div className='flex items-center gap-12px shrink-0'>
        {!props.hideAdvancedControls && conversation_id != null && (
          <>
            <AutoWorkControl target={{ kind: 'conversation', id: conversation_id }} />
            <IdmmControl target={{ kind: 'conversation', id: conversation_id }} />
            {(props.knowledgeEnabled ?? true) && (
              <KnowledgeControl target={{ kind: 'conversation', id: conversation_id }} />
            )}
          </>
        )}
        {props.headerExtra}
      </div>
    </ArcoLayout.Header>
  );

  const headerBlock = (
    <>
      {desktopHeader}
      {props.tabsSlot}
    </>
  );

  return (
    <ArcoLayout
      className='size-full color-black '
      style={{
        // fontFamily: `cursive,"anthropicSans","anthropicSans Fallback",system-ui,Segoe UI,Roboto,Helvetica,Arial,sans-serif`,
      }}
    >
      <div ref={containerRef} className='flex flex-1 relative w-full overflow-hidden'>
        {/* Unified layout: single DOM structure prevents children unmount/remount on preview toggle */}
        <div
          className='flex flex-col min-w-0'
          style={{
            flexGrow: 1,
            flexShrink: 1,
            flexBasis: 0,
          }}
        >
          <div className='shrink-0 !bg-1'>{headerBlock}</div>
          <div className='flex flex-1 min-h-0 relative'>
            {/* Chat area - always mounted, never unmounted on preview toggle */}
            <div
              className='flex flex-col relative'
              style={{
                flexGrow: isPreviewOpen ? 0 : 1,
                flexShrink: 0,
                flexBasis: isPreviewOpen ? `${chatFlex}%` : 0,
                minWidth: '240px',
              }}
            >
              <ArcoLayout.Content className='flex flex-col flex-1 bg-1 overflow-hidden'>
                {props.children}
              </ArcoLayout.Content>
            </div>
            {/* Preview panel - conditionally rendered */}
            {isPreviewOpen && (
              <div
                className={classNames(
                  'preview-panel flex flex-col relative overflow-visible rounded-[15px]',
                  'mb-[12px] mr-[12px] ml-[8px]'
                )}
                style={{
                  flexGrow: 1,
                  flexShrink: 1,
                  flexBasis: 0,
                  border: '1px solid var(--bg-3)',
                  minWidth: '260px',
                  boxSizing: 'border-box',
                }}
              >
                {createPreviewDragHandle({
                  className: 'absolute top-0 bottom-0 z-30',
                  style: { width: '20px', left: '-20px' },
                  linePlacement: 'end',
                  lineClassName: 'opacity-30 group-hover:opacity-100 group-active:opacity-100',
                  lineStyle: { width: '2px' },
                })}
                <div className='h-full w-full overflow-hidden rounded-[15px]'>
                  <PreviewPanel />
                </div>
              </div>
            )}
          </div>
        </div>
        {workspaceEnabled && (
          <div
            className={classNames('!bg-1 relative chat-layout-right-sider layout-sider')}
            style={{
              flexGrow: 0,
              flexShrink: 0,
              flexBasis: rightSiderCollapsed ? '0px' : `${Math.round(workspaceWidthPx)}px`,
              width: rightSiderCollapsed ? '0px' : `${Math.round(workspaceWidthPx)}px`,
              minWidth: rightSiderCollapsed ? '0px' : `${MIN_WORKSPACE_PANEL_PX}px`,
              overflow: 'hidden',
              borderLeft: rightSiderCollapsed ? 'none' : '1px solid var(--bg-3)',
            }}
          >
            {!rightSiderCollapsed &&
              createWorkspaceDragHandle({ className: 'absolute left-0 top-0 bottom-0', style: {}, reverse: true })}
            <WorkspacePanelHeader
              showToggle={Boolean(props.selfContainedWorkspaceToggle) || (!isMacRuntime && !isWindowsRuntime)}
              collapsed={rightSiderCollapsed}
              onToggle={() => workspaceTarget && dispatchWorkspaceToggleEvent(workspaceTarget)}
              togglePlacement='right'
              workspacePath={workspacePath}
              isTemporaryWorkspace={isTemporaryWorkspace}
              conversation_id={conversation_id}
              activeTab={activeWorkspaceTab}
            >
              {activeWorkspaceTitle}
            </WorkspacePanelHeader>
            <ArcoLayout.Content style={{ height: `calc(100% - ${WORKSPACE_HEADER_HEIGHT}px)` }}>
              {props.sider}
            </ArcoLayout.Content>
          </div>
        )}
        {workspaceEnabled && (
          <WorkspaceToolRail
            t={t}
            activeTab={activeWorkspaceTab}
            expanded={!rightSiderCollapsed}
            onSelect={selectWorkspaceTool}
            changeCount={workspaceChangeCount}
            extraTabs={props.workspaceExtraTabs}
            collaboration={workspaceCollaboration}
            footer={
              <button
                type='button'
                className='workspace-tool-rail__item workspace-tool-rail__item--collapse'
                onClick={() => {
                  if (rightSiderCollapsed && props.workspaceCollaboration?.active) {
                    props.workspaceCollaboration.onClick();
                  }
                  persistRightSiderCollapsed(!rightSiderCollapsed);
                }}
                aria-label={
                  rightSiderCollapsed
                    ? t('conversation.workspace.expand', { defaultValue: '展开侧栏' })
                    : t('conversation.workspace.collapse', { defaultValue: '收起侧栏' })
                }
              >
                {rightSiderCollapsed ? <span>‹</span> : <span>›</span>}
              </button>
            }
          />
        )}
      </div>
    </ArcoLayout>
  );
};

/**
 * ChatLayout — per-surface chat layout. Mounts its OWN {@link PreviewProvider}
 * so the file/diff preview state is scoped to this surface instead of a global
 * singleton (which used to leak preview tabs across views and required the
 * three `closePreview()` cleanup calls in Sider / ConversationShell /
 * conversation index that have since been removed). The provider unmounts with
 * the surface, so cross-view leak can no longer happen.
 *
 * The persistence namespace includes the conversation id, so switching
 * conversations restores only that conversation's tabs. `subscribeGlobalOpen`
 * lets agent/MCP `preview.open` events open a preview on the conversation
 * surface (the primary surface).
 */
const ChatLayout: React.FC<ChatLayoutProps> = (props) => {
  const pendingPreviewScope = useRef(`conversation-pending:${uuid()}`);
  const previewScope = props.conversation_id
    ? browserStorageKey('workspace-preview', 'conversation', props.conversation_id)
    : pendingPreviewScope.current;

  return (
    <PreviewProvider
      key={previewScope}
      persistNamespace={previewScope}
      subscribeGlobalOpen={true}
    >
      <ChatLayoutInner {...props} />
    </PreviewProvider>
  );
};

export default ChatLayout;
