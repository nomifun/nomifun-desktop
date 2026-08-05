/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { Message, Modal, Spin } from '@arco-design/web-react';
import { AddOne, Left, Pic } from '@icon-park/react';
import classNames from 'classnames';
import { ipcBridge } from '@/common';
import { useResizableSplit } from '@/renderer/hooks/ui/useResizableSplit';
import { useContainerWidth } from '@/renderer/hooks/ui/useContainerWidth';
import { useLayoutContext } from '@/renderer/hooks/context/LayoutContext';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import CompanionSidebar from './CompanionSidebar';
import CreateCompanionModal from './CompanionSidebar/CreateCompanionModal';
import FigureLibraryPage from './FigureLibraryPage';
import { AsideHost } from './workspace/AsideHost';
import WorkspaceHeader from './workspace/WorkspaceHeader';
import { WORKSPACE_TABS, isWorkspaceTabKey } from './workspace/types';
import type { WorkspaceTabKey } from './workspace/types';
import OverviewTab from './workspace/tabs/OverviewTab';
import MemoryTab from './workspace/tabs/MemoryTab';
import RemoteTab from './workspace/tabs/RemoteTab';
import EvolutionTab from './workspace/tabs/EvolutionTab';
import SkillsTab from './workspace/tabs/SkillsTab';
import HistoryTab from './workspace/tabs/HistoryTab';
import OtherTab from './workspace/tabs/OtherTab';
import { useCompanion, useCompanions } from './useNomi';
import type { ICompanionProfile, ICompanionWithStatus } from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';

const SIDER_STORAGE_KEY = 'nomifun:nomi-sider-width';

type AttentionFlags = Partial<Record<WorkspaceTabKey, boolean>>;
const EMPTY_ATTENTION: AttentionFlags = {};

const TAB_COMPONENTS: Record<WorkspaceTabKey, React.ComponentType<import('./workspace/types').WorkspaceTabProps>> = {
  overview: OverviewTab,
  memory: MemoryTab,
  remote: RemoteTab,
  evolution: EvolutionTab,
  skills: SkillsTab,
  history: HistoryTab,
  other: OtherTab,
};

/**
 * 桌面伙伴 (desktop companion) management workspace.
 *
 * Three regions, each with one job: the left sidebar answers "which companion",
 * the centre answers "what about them", and the right pane — when a tab opens it
 * — answers "this one thing in detail". State lives in the URL
 * (`?companion=&tab=&view=`) so any surface can deep-link into it.
 *
 * Replaces the previous two-level Radio.Group design, whose outer 伙伴/共享/形象库
 * "domain" switch existed only because half the settings were install-global.
 */
const NomiWorkspacePage: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const layout = useLayoutContext();
  const isMobile = layout?.isMobile ?? false;
  const [searchParams, setSearchParams] = useSearchParams();
  const { companions, loading, refresh } = useCompanions();

  const [createOpen, setCreateOpen] = useState(false);
  const [attention, setAttention] = useState<{ owner: string; flags: AttentionFlags }>({
    owner: '',
    flags: EMPTY_ATTENTION,
  });

  const resize = useResizableSplit({
    unit: 'px',
    defaultWidth: 248,
    minWidth: 200,
    maxWidth: 360,
    storageKey: SIDER_STORAGE_KEY,
  });

  // Pane padding follows the PANE width, not the viewport: inside a three-column
  // shell the content column is far narrower than the window.
  const { ref: paneRef, width: paneWidth } = useContainerWidth<HTMLDivElement>();
  const panePadX = paneWidth === 0 ? 'px-24px' : paneWidth >= 600 ? 'px-40px' : paneWidth >= 420 ? 'px-24px' : 'px-16px';

  const figuresActive = searchParams.get('view') === 'figures';
  const tabParam = searchParams.get('tab');
  const activeTab: WorkspaceTabKey = isWorkspaceTabKey(tabParam) ? tabParam : 'overview';

  const companionParam = searchParams.get('companion');
  const selectedCompanionId = useMemo(() => {
    if (companionParam) {
      const matched = companions.find((c) => c.companion_id === companionParam);
      if (matched) return matched.companion_id;
    }
    return companions[0]?.companion_id ?? null;
  }, [companionParam, companions]);

  const companion = useCompanion(selectedCompanionId);
  const attentionOwner = selectedCompanionId ?? '';

  const setTab = useCallback(
    (key: WorkspaceTabKey) => {
      setSearchParams(
        (prev) => {
          prev.set('tab', key);
          prev.delete('view');
          return prev;
        },
        { replace: true }
      );
    },
    [setSearchParams]
  );

  const selectCompanion = useCallback(
    (id: CompanionId) => {
      setSearchParams(
        (prev) => {
          prev.set('companion', id);
          prev.delete('view');
          return prev;
        },
        { replace: true }
      );
    },
    [setSearchParams]
  );

  const openFigures = useCallback(() => {
    setSearchParams(
      (prev) => {
        prev.set('view', 'figures');
        return prev;
      },
      { replace: true }
    );
  }, [setSearchParams]);

  const handleCreated = useCallback(
    async (profile: ICompanionProfile) => {
      await refresh();
      // One atomic update: two sequential functional setSearchParams calls would
      // both read the pre-navigation params and the second would drop the first's
      // `companion=` change.
      setSearchParams(
        (prev) => {
          prev.set('companion', profile.companion_id);
          prev.set('tab', 'overview');
          prev.delete('view');
          return prev;
        },
        { replace: true }
      );
    },
    [refresh, setSearchParams]
  );

  const handleDeleted = useCallback(
    (deletedId: CompanionId) => {
      const rest = companions.filter((c) => c.companion_id !== deletedId);
      setSearchParams(
        (prev) => {
          if (rest[0]) prev.set('companion', rest[0].companion_id);
          else prev.delete('companion');
          return prev;
        },
        { replace: true }
      );
      void refresh();
    },
    [companions, refresh, setSearchParams]
  );

  const requestDelete = useCallback(
    (target: ICompanionWithStatus) => {
      Modal.confirm({
        title: t('nomi.settings.deleteConfirmTitle'),
        content: t('nomi.danger.deleteConfirmBody', {
          companionName: target.name,
          defaultValue:
            '将永久删除「{{companionName}}」及其全部记忆、技能、成长进度与聊天记录，无法恢复。注意：迁移包可以带走设定、记忆和技能，但聊天记录不在包内，先导出也留不下它。',
        }),
        okButtonProps: { status: 'danger' },
        onOk: async () => {
          try {
            await ipcBridge.companion.deleteCompanion.invoke({ companion_id: target.companion_id });
            Message.success(t('nomi.settings.deleted', { companionName: target.name }));
            handleDeleted(target.companion_id);
          } catch (error) {
            Message.error(String(error));
          }
        },
      });
    },
    [handleDeleted, t]
  );

  /** Persist a new roster order by writing each companion's position. */
  const handleReorder = useCallback(
    (orderedIds: CompanionId[]) => {
      void (async () => {
        try {
          await Promise.all(
            orderedIds.map((id, index) =>
              ipcBridge.companion.patchCompanion.invoke({ companion_id: id, patch: { order_index: index } })
            )
          );
        } catch (error) {
          Message.error(String(error));
        } finally {
          // Refresh either way: a partial failure must not leave the UI showing an
          // order the backend did not accept.
          void refresh();
        }
      })();
    },
    [refresh]
  );

  const openChat = useCallback(async () => {
    if (!selectedCompanionId) return;
    try {
      const thread = await ipcBridge.companion.ensureCompanionSession.invoke({ companion_id: selectedCompanionId });
      void navigate(`/conversation/${thread.conversation_id}`);
    } catch {
      // A companion with no model configured cannot mint a session — keep the
      // user here, where they can configure it.
      Message.info(t('nomi.chat.modelMissing'));
    }
  }, [navigate, selectedCompanionId, t]);

  const reportAttention = useMemo(
    () =>
      Object.fromEntries(
        WORKSPACE_TABS.map((key) => [
          key,
          (hasAttention: boolean) =>
            setAttention((prev) =>
              prev.owner === attentionOwner && prev.flags[key] === hasAttention
                ? prev
                : { owner: attentionOwner, flags: { ...(prev.owner === attentionOwner ? prev.flags : {}), [key]: hasAttention } }
            ),
        ])
      ) as Record<WorkspaceTabKey, (hasAttention: boolean) => void>,
    [attentionOwner]
  );

  const closeFigures = useCallback(() => {
    setSearchParams(
      (prev) => {
        prev.delete('view');
        return prev;
      },
      { replace: true }
    );
  }, [setSearchParams]);

  // Attention dots belong to a companion, not to the page: reading them through
  // the owner id means a stale dot cannot follow the user onto another companion,
  // without needing to reset state on switch.
  const attentionFlags = attention.owner === attentionOwner ? attention.flags : EMPTY_ATTENTION;

  // Every hook must be above this early return: React counts hooks per render, so
  // a hook declared below it is skipped on the loading pass and reached on the
  // loaded one — "Rendered more hooks than during the previous render", which
  // corrupts the root and makes unrelated pages fail too.
  if (loading) {
    return (
      <div className='flex size-full items-center justify-center'>
        <Spin />
      </div>
    );
  }

  const ActiveTab = TAB_COMPONENTS[activeTab];

  const workspace = figuresActive ? (
    <>
      {/* 形象库 takes over the workspace rather than opening a route of its own, so
          it needs its own way back — on mobile there is no sidebar to click. */}
      <div className={classNames('shrink-0 pt-20px pb-12px', panePadX)}>
        <div className='mx-auto w-full max-w-1100px box-border flex items-center gap-8px'>
          <div
            role='button'
            tabIndex={0}
            onClick={closeFigures}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                closeFigures();
              }
            }}
            className='flex items-center gap-4px h-28px rd-8px px-8px cursor-pointer text-13px text-t-secondary hover:text-t-primary hover:bg-fill-2 transition-colors outline-none'
          >
            <Left theme='outline' size='14' fill='currentColor' />
            {t('nomi.figures.back', { defaultValue: '返回伙伴' })}
          </div>
          <span className='text-18px leading-24px font-600 text-t-primary'>{t('nomi.customFigure.libraryTitle')}</span>
        </div>
      </div>
      <div className='flex-1 min-h-0 overflow-y-auto'>
        <div className={classNames('mx-auto w-full max-w-1100px box-border pb-32px', panePadX)}>
          <FigureLibraryPage />
        </div>
      </div>
    </>
  ) : selectedCompanionId ? (
    <>
      <div className={classNames('shrink-0 pt-20px pb-12px', panePadX)}>
        <div className='mx-auto w-full max-w-1100px box-border'>
          {isMobile && (
            <div className='mb-12px flex items-center gap-8px'>
              {companions.length > 1 && (
                <NomiSelect
                  className='flex-1 min-w-0'
                  value={selectedCompanionId}
                  onChange={(id: CompanionId) => selectCompanion(id)}
                >
                  {companions.map((c) => (
                    <NomiSelect.Option key={c.companion_id} value={c.companion_id}>
                      {c.name}
                    </NomiSelect.Option>
                  ))}
                </NomiSelect>
              )}
              {/* The sidebar is hidden on mobile, so its two entries need to exist
                  here or 新建伙伴 / 形象库 become unreachable. */}
              <div
                role='button'
                tabIndex={0}
                aria-label={t('nomi.companions.create')}
                onClick={() => setCreateOpen(true)}
                className='shrink-0 flex items-center justify-center w-32px h-32px rd-8px cursor-pointer text-t-secondary hover:text-t-primary hover:bg-fill-2 transition-colors outline-none'
              >
                <AddOne theme='outline' size='16' fill='currentColor' strokeWidth={3} />
              </div>
              <div
                role='button'
                tabIndex={0}
                aria-label={t('nomi.customFigure.libraryTitle')}
                onClick={openFigures}
                className='shrink-0 flex items-center justify-center w-32px h-32px rd-8px cursor-pointer text-t-secondary hover:text-t-primary hover:bg-fill-2 transition-colors outline-none'
              >
                <Pic theme='outline' size='16' fill='currentColor' strokeWidth={3} />
              </div>
            </div>
          )}
          <WorkspaceHeader
            companion={companion}
            activeTab={activeTab}
            onTabChange={setTab}
            attention={attentionFlags}
            onOpenChat={() => void openChat()}
          />
        </div>
      </div>
      <div className='flex-1 min-h-0 overflow-y-auto'>
        <div className={classNames('mx-auto w-full max-w-1100px box-border pb-32px', panePadX)}>
          <ActiveTab
            key={`${selectedCompanionId}:${activeTab}`}
            companionId={selectedCompanionId}
            companion={companion}
            onAttentionChange={reportAttention[activeTab]}
          />
        </div>
      </div>
    </>
  ) : (
    <div className='flex-1 flex flex-col items-center justify-center gap-14px py-64px px-24px text-center'>
      <span className='flex items-center justify-center w-72px h-72px rd-full bg-fill-2 text-primary-6'>
        <AddOne theme='outline' size='30' fill='currentColor' strokeWidth={3} />
      </span>
      <span className='text-16px font-500 text-t-primary'>
        {t('nomi.companions.emptyTitle', { defaultValue: '还没有桌面伙伴' })}
      </span>
      <span className='max-w-360px text-13px leading-20px text-t-tertiary'>
        {t('nomi.companions.emptyHint', {
          defaultValue: '创建一个伙伴，给它一个名字和形象，然后配置模型就可以开始对话了。',
        })}
      </span>
      <div
        role='button'
        tabIndex={0}
        onClick={() => setCreateOpen(true)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            setCreateOpen(true);
          }
        }}
        className='mt-2px flex items-center gap-6px rd-full px-18px py-9px cursor-pointer font-700 text-13px text-[var(--color-text-1)] bg-[rgba(var(--primary-6),0.12)] hover:bg-[rgba(var(--primary-6),0.18)] shadow-[0_6px_18px_rgba(var(--primary-6),0.14)] transition-colors outline-none'
      >
        {t('nomi.companions.create')}
      </div>
    </div>
  );

  return (
    <>
      {/* AsideHost's landing slot must sit INSIDE this flex row so a portalled
          detail pane becomes a third column rather than a sibling of the row. */}
      <div className='relative flex size-full min-h-0'>
        <AsideHost>
          {!isMobile && (
            <CompanionSidebar
              companions={companions}
              selectedId={selectedCompanionId}
              figuresActive={figuresActive}
              width={resize.splitRatio}
              onSelect={selectCompanion}
              onOpenFigures={openFigures}
              onCreate={() => setCreateOpen(true)}
              onRequestDelete={requestDelete}
              onReorder={handleReorder}
              resizeHandle={resize.createDragHandle({ className: 'right-0' })}
            />
          )}
          <div ref={paneRef} className='flex-1 min-w-0 min-h-0 flex flex-col'>
            {workspace}
          </div>
        </AsideHost>
      </div>
      <CreateCompanionModal visible={createOpen} onCancel={() => setCreateOpen(false)} onCreated={handleCreated} />
    </>
  );
};

export default NomiWorkspacePage;
