/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { SessionTarget } from '@/common/types/ids';
import type { WorkspaceExtraTab, WorkspaceTab } from '@/renderer/pages/conversation/Workspace/types';
import { Tooltip } from '@arco-design/web-react';
import { Branch, Change, ChartHistogram, Earth, FolderOpen } from '@icon-park/react';
import classNames from 'classnames';
import type { TFunction } from 'i18next';
import React from 'react';
// Every `.workspace-tool-rail*` rule lives in this stylesheet. It used to be
// imported only by ChatLayout, so the terminal page — which renders this rail
// without ever loading ChatLayout, in its own React.lazy chunk — showed an
// unstyled rail: horizontal buttons with the labels that should be
// visually-hidden. Importing it here means every consumer of the rail gets the
// rules that describe it.
import './chat-layout.css';

export const WORKSPACE_PANEL_TAB_EVENT = 'nomifun-workspace-panel-tab';
export const WORKSPACE_PANEL_META_EVENT = 'nomifun-workspace-panel-meta';

export interface WorkspacePanelTabDetail {
  tab: WorkspaceTab;
  target: SessionTarget;
}

export interface WorkspacePanelMetaDetail {
  target: SessionTarget;
  changeCount: number;
}

export function dispatchWorkspacePanelTabEvent(tab: WorkspaceTab, target: SessionTarget) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(
    new CustomEvent<WorkspacePanelTabDetail>(WORKSPACE_PANEL_TAB_EVENT, { detail: { tab, target } })
  );
}

export function isWorkspacePanelEventForTarget(
  eventTarget: SessionTarget | undefined,
  target: SessionTarget | undefined
): boolean {
  return Boolean(
    eventTarget && target && eventTarget.kind === target.kind && eventTarget.id === target.id
  );
}

export function dispatchWorkspacePanelMetaEvent(detail: WorkspacePanelMetaDetail) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent<WorkspacePanelMetaDetail>(WORKSPACE_PANEL_META_EVENT, { detail }));
}

export type WorkspaceToolRailCollaboration = {
  active: boolean;
  available: boolean;
  statusColor?: string;
  onClick: () => void;
};

export type SessionBrowserTool = {
  active: boolean;
  label: React.ReactNode;
  controls: string;
  buttonRef?: React.Ref<HTMLButtonElement>;
  onClick: () => void;
};

type WorkspaceToolRailProps = {
  t: TFunction;
  workspaceAvailable?: boolean;
  activeTab: WorkspaceTab;
  expanded: boolean;
  onSelect: (tab: WorkspaceTab) => void;
  changeCount?: number;
  extraTabs?: WorkspaceExtraTab[];
  collaboration?: WorkspaceToolRailCollaboration;
  browser?: SessionBrowserTool;
  footer?: React.ReactNode;
};

type ToolRailItemProps = {
  active: boolean;
  label: React.ReactNode;
  icon: React.ReactNode;
  badge?: React.ReactNode;
  statusColor?: string;
  controls?: string;
  buttonRef?: React.Ref<HTMLButtonElement>;
  onClick: () => void;
};

const ToolRailItem: React.FC<ToolRailItemProps> = ({ active, label, icon, badge, statusColor, controls, buttonRef, onClick }) => (
  <Tooltip position='left' content={label} mini className='workspace-tool-rail__tooltip'>
    <button
      ref={buttonRef}
      type='button'
      className={classNames('workspace-tool-rail__item', {
        'workspace-tool-rail__item--active': active,
      })}
      aria-pressed={active}
      aria-expanded={controls ? active : undefined}
      aria-controls={controls}
      onClick={onClick}
    >
      <span className='workspace-tool-rail__icon'>{icon}</span>
      <span className='workspace-tool-rail__label'>{label}</span>
      {statusColor && <span className='workspace-tool-rail__status' style={{ background: statusColor }} />}
      {badge}
    </button>
  </Tooltip>
);

const WorkspaceToolRail: React.FC<WorkspaceToolRailProps> = ({
  t,
  workspaceAvailable = true,
  activeTab,
  expanded,
  onSelect,
  changeCount = 0,
  extraTabs,
  collaboration,
  browser,
  footer,
}) => (
  <aside
    className='workspace-tool-rail'
    aria-label={t('conversation.workspace.toolsLabel', { defaultValue: 'Session tools' })}
  >
    {workspaceAvailable && <ToolRailItem
      active={expanded && activeTab === 'files'}
      label={t('conversation.workspace.changes.filesTab')}
      icon={<FolderOpen size={18} />}
      onClick={() => onSelect('files')}
    />}
    {workspaceAvailable && <ToolRailItem
      active={expanded && activeTab === 'changes'}
      label={t('conversation.workspace.changes.tab')}
      icon={<Change size={18} />}
      badge={changeCount > 0 ? <span className='workspace-tool-rail__badge' /> : undefined}
      onClick={() => onSelect('changes')}
    />}
    {workspaceAvailable && extraTabs?.map((tab) => (
      <ToolRailItem
        key={tab.key}
        active={expanded && activeTab === tab.key}
        label={tab.title}
        icon={tab.icon ?? <ChartHistogram size={18} />}
        onClick={() => onSelect(tab.key)}
      />
    ))}
    {collaboration?.available && (
      <>
        <span className='workspace-tool-rail__divider' />
        <ToolRailItem
          active={collaboration.active}
          label={t('agentExecution.panel.title', { defaultValue: '协作任务' })}
          icon={<Branch size={18} />}
          statusColor={collaboration.statusColor}
          onClick={collaboration.onClick}
        />
      </>
    )}
    {browser && (
      <>
        {(workspaceAvailable || collaboration?.available) && <span className='workspace-tool-rail__divider' />}
        <ToolRailItem
          active={browser.active}
          label={browser.label}
          icon={<Earth size={18} />}
          controls={browser.controls}
          buttonRef={browser.buttonRef}
          onClick={browser.onClick}
        />
      </>
    )}
    {footer && <div className='workspace-tool-rail__footer'>{footer}</div>}
  </aside>
);

export default WorkspaceToolRail;
