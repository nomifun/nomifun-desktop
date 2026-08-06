/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@arco-design/web-react';
import SegmentedTabs from '@/renderer/components/base/SegmentedTabs';
import type { SegmentedTabItem } from '@/renderer/components/base/SegmentedTabs';
import CompanionAvatar from '@renderer/pages/companion/CompanionAvatar';
import { customFigureMetaOf } from '@renderer/pages/companion/characters/customMeta';
import type { CompanionMood } from '@renderer/pages/companion/characters';
import { WORKSPACE_TABS } from './types';
import type { CompanionHandle, WorkspaceTabKey } from './types';

interface Props {
  companion: CompanionHandle;
  activeTab: WorkspaceTabKey;
  onTabChange: (key: WorkspaceTabKey) => void;
  /** Tabs currently reporting something awaiting the user. */
  attention: Partial<Record<WorkspaceTabKey, boolean>>;
  onOpenChat: () => void;
}

const TAB_LABEL_KEYS: Record<WorkspaceTabKey, { key: string; zh: string }> = {
  overview: { key: 'nomi.tabs.overview', zh: '总览' },
  memory: { key: 'nomi.tabs.memory', zh: '记忆&知识库' },
  remote: { key: 'nomi.tabs.remote', zh: '远程控制' },
  evolution: { key: 'nomi.tabs.evolution', zh: '进化' },
  skills: { key: 'nomi.tabs.skills', zh: '技能' },
  history: { key: 'nomi.tabs.history', zh: '聊天历史' },
  other: { key: 'nomi.tabs.other', zh: '其他' },
};

/**
 * The workspace header: who you are configuring, then what about them.
 *
 * Identity sits above the tab strip rather than beside it, so switching tabs
 * never moves the name — the page always answers "which companion" in the same
 * place. The old page repeated the tab strip's job with a second Radio.Group
 * one row above it; here the only control at this level is the tab strip.
 */
const WorkspaceHeader: React.FC<Props> = ({ companion, activeTab, onTabChange, attention, onOpenChat }) => {
  const { t } = useTranslation();
  const { profile, status } = companion;

  const items: SegmentedTabItem[] = WORKSPACE_TABS.map((key) => ({
    key,
    label: t(TAB_LABEL_KEYS[key].key, { defaultValue: TAB_LABEL_KEYS[key].zh }),
    dot: attention[key] === true,
  }));

  return (
    <div className='shrink-0 flex flex-col gap-12px'>
      <div className='flex items-center gap-10px min-w-0'>
        {profile && (
          <CompanionAvatar
            character={profile.character}
            companionId={profile.companion_id}
            customFigure={customFigureMetaOf(profile)}
            mood={(status?.mood as CompanionMood) || 'content'}
            activity='idle'
            size={34}
          />
        )}
        <div className='min-w-0 flex-1'>
          <div className='text-18px leading-24px font-600 text-t-primary truncate'>{profile?.name ?? ''}</div>
          {status && (
            <div className='text-12px leading-18px text-t-tertiary'>
              Lv{status.level} · {t(`nomi.levels.l${Math.min(status.level, 5)}`)}
            </div>
          )}
        </div>
        {/* Chat lives in /conversation — this is the one path from管理 into it,
            so it stays a visible primary action rather than hiding in a menu. */}
        <Button type='primary' shape='round' size='small' className='shrink-0' onClick={onOpenChat}>
          {t('nomi.openChat')}
        </Button>
      </div>
      <SegmentedTabs
        items={items}
        activeKey={activeTab}
        onChange={(key) => onTabChange(key as WorkspaceTabKey)}
        size='sm'
      />
    </div>
  );
};

export default WorkspaceHeader;
