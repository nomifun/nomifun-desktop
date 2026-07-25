/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Button, Tag } from '@arco-design/web-react';
import { Delete } from '@icon-park/react';
import classNames from 'classnames';
import { useTranslation } from 'react-i18next';
import type { IBrowserLane, IBrowserTab } from '@/common/browser/browserTypes';
import type { BrowserConversationGroup } from './browserInventoryModel';

interface BrowserInventoryTreeProps {
  groups: BrowserConversationGroup[];
  selectedLaneId: string | null;
  currentConversationId?: string | null;
  busyLaneId?: string | null;
  busyConversationId?: string | null;
  onSelectLane: (lane: IBrowserLane) => void;
  onCloseLane: (lane: IBrowserLane) => void;
  onCloseConversation: (group: BrowserConversationGroup) => void;
}

const stateColor = (state: string): string => {
  if (state === 'running') return 'green';
  if (state === 'queued') return 'orange';
  if (state === 'failed') return 'red';
  if (state === 'frozen') return 'arcoblue';
  return 'gray';
};

const shortUrl = (lane: IBrowserLane, noActivePage: string): string => {
  const value = lane.url || lane.tabs.find((tab) => tab.active)?.url || lane.tabs[0]?.url;
  if (!value) return noActivePage;
  try {
    return new URL(value).hostname || value;
  } catch {
    return value;
  }
};

const tabIsActive = (lane: IBrowserLane, tab: IBrowserTab): boolean =>
  tab.tab_id === lane.active_tab_id || tab.active === true;

const tabTitle = (tab: IBrowserTab): string => tab.title?.trim() || tab.url?.trim() || tab.tab_id;

const tabUrl = (tab: IBrowserTab, noActivePage: string): string =>
  tab.url?.trim() || noActivePage;

const BrowserInventoryTree: React.FC<BrowserInventoryTreeProps> = ({
  groups,
  selectedLaneId,
  currentConversationId,
  busyLaneId,
  busyConversationId,
  onSelectLane,
  onCloseLane,
  onCloseConversation,
}) => {
  const { t } = useTranslation();
  const lifecycleLabel = (state: string): string => {
    switch (state) {
      case 'queued':
        return t('browser.state.lifecycle.queued');
      case 'starting':
        return t('browser.state.lifecycle.starting');
      case 'running':
        return t('browser.state.lifecycle.running');
      case 'frozen':
        return t('browser.state.lifecycle.frozen');
      case 'stopping':
        return t('browser.state.lifecycle.stopping');
      case 'failed':
        return t('browser.state.lifecycle.failed');
      default:
        return state;
    }
  };

  return (
    <div className='flex flex-col gap-10px'>
      {groups.map((group) => (
        <section
          key={group.key}
          className='border border-solid border-[var(--color-border-2)] rd-10px overflow-hidden bg-bg-1'
        >
        <div className='flex items-center gap-8px px-10px py-8px bg-fill-1 border-b border-solid border-[var(--color-border-2)] border-t-0 border-l-0 border-r-0'>
          <div className='min-w-0 flex-1'>
            <div className='flex items-center gap-6px'>
              <span className='truncate text-13px font-600'>{group.label}</span>
              {group.conversationId === currentConversationId && (
                <Tag size='small' color='arcoblue'>
                  {t('browser.tree.current')}
                </Tag>
              )}
            </div>
            <div className='mt-2px text-11px text-t-tertiary'>
              {t('browser.tree.summary', {
                running: group.runningCount,
                queued: group.queuedCount,
              })}
            </div>
          </div>
          {group.conversationId && (
            <Button
              type='text'
              size='mini'
              status='danger'
              loading={busyConversationId === group.conversationId}
              aria-label={t('browser.tree.closeConversationAria', { name: group.label })}
              icon={<Delete theme='outline' size='13' />}
              onClick={() => onCloseConversation(group)}
            />
          )}
        </div>

        <div className='p-6px flex flex-col gap-8px'>
          {group.owners.map((owner) => (
            <div key={owner.key}>
              <div className='px-5px pb-4px text-11px font-500 text-t-secondary truncate'>
                {owner.label}
              </div>
              <div className='flex flex-col gap-4px'>
                {owner.lanes.map((lane) => {
                  const active = selectedLaneId === lane.lane_id;
                  const title =
                    lane.title ||
                    lane.tabs.find((tab) => tab.tab_id === lane.active_tab_id)?.title ||
                    lane.tabs.find((tab) => tab.active)?.title ||
                    lane.lane_name ||
                    t('browser.tree.laneFallback');
                  return (
                    <div key={lane.lane_id} data-browser-lane-id={lane.lane_id}>
                      <div
                        className={classNames(
                          'group flex items-center gap-6px rd-8px px-8px py-7px cursor-pointer transition-colors',
                          active
                            ? 'bg-primary-1 text-primary-6'
                            : 'hover:bg-fill-2 text-t-primary'
                        )}
                        role='button'
                        tabIndex={0}
                        onClick={() => onSelectLane(lane)}
                        onKeyDown={(event) => {
                          if (event.key === 'Enter' || event.key === ' ') {
                            event.preventDefault();
                            onSelectLane(lane);
                          }
                        }}
                      >
                        <span
                          className={classNames(
                            'size-7px rd-full shrink-0',
                            lane.lifecycle_state === 'running'
                              ? 'bg-green-6'
                              : lane.lifecycle_state === 'queued'
                                ? 'bg-orange-6'
                                : lane.lifecycle_state === 'failed'
                                  ? 'bg-red-6'
                                  : 'bg-gray-5'
                          )}
                        />
                        <div className='min-w-0 flex-1'>
                          <div className='flex items-center gap-5px'>
                            <span className='truncate text-12px font-500'>{title}</span>
                            <Tag size='small' color={stateColor(lane.lifecycle_state)}>
                              {lifecycleLabel(lane.lifecycle_state)}
                            </Tag>
                          </div>
                          <div className='truncate mt-2px text-11px text-t-tertiary'>
                            {shortUrl(lane, t('browser.tree.noActivePage'))}
                            {lane.queue?.position
                              ? ` · ${t('browser.tree.queuePosition', {
                                  position: lane.queue.position,
                                })}`
                              : ''}
                          </div>
                        </div>
                        <Button
                          type='text'
                          size='mini'
                          status='danger'
                          className={active ? '' : 'opacity-0 group-hover:opacity-100'}
                          loading={busyLaneId === lane.lane_id}
                          aria-label={t('browser.tree.closeLaneAria', { name: title })}
                          icon={<Delete theme='outline' size='12' />}
                          onClick={(event) => {
                            event.stopPropagation();
                            onCloseLane(lane);
                          }}
                        />
                      </div>

                      {lane.tabs.length > 0 && (
                        <div
                          className='ml-15px mt-2px pl-10px border-l border-solid border-[var(--color-border-2)] flex flex-col gap-2px'
                          role='list'
                          data-browser-lane-tabs={lane.lane_id}
                        >
                          {lane.tabs.map((tab) => {
                            const activeTab = tabIsActive(lane, tab);
                            return (
                              <div
                                key={tab.tab_id}
                                className={classNames(
                                  'min-w-0 rd-6px px-7px py-5px',
                                  activeTab ? 'bg-primary-1' : 'hover:bg-fill-1'
                                )}
                                role='listitem'
                                data-browser-tab-id={tab.tab_id}
                                data-browser-tab-active={activeTab ? 'true' : 'false'}
                                data-browser-tab-crashed={tab.crashed ? 'true' : 'false'}
                              >
                                <div className='min-w-0 flex items-center gap-5px'>
                                  <span
                                    className={classNames(
                                      'size-5px rd-full shrink-0',
                                      tab.crashed
                                        ? 'bg-red-6'
                                        : activeTab
                                          ? 'bg-primary-6'
                                          : 'bg-gray-4'
                                    )}
                                  />
                                  <span
                                    className={classNames(
                                      'truncate text-11px',
                                      activeTab
                                        ? 'font-600 text-primary-6'
                                        : 'font-500 text-t-secondary'
                                    )}
                                  >
                                    {tabTitle(tab)}
                                  </span>
                                  {activeTab && (
                                    <Tag size='small' color='arcoblue'>
                                      {t('browser.tree.current')}
                                    </Tag>
                                  )}
                                  {tab.crashed && (
                                    <Tag size='small' color='red'>
                                      {t('browser.viewer.crashed')}
                                    </Tag>
                                  )}
                                </div>
                                <div
                                  className={classNames(
                                    'truncate ml-10px mt-1px text-10px',
                                    tab.crashed ? 'text-red-6' : 'text-t-tertiary'
                                  )}
                                >
                                  {tabUrl(tab, t('browser.tree.noActivePage'))}
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          ))}
        </div>
        </section>
      ))}
    </div>
  );
};

export default BrowserInventoryTree;
