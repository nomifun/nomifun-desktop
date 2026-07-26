/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Alert, Button, Tag } from '@arco-design/web-react';
import { Delete } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import type { BrowserDisplayMode } from '@/common/browser/browserSettings';
import type { IBrowserLane } from '@/common/browser/browserTypes';
import EmbeddedBrowserViewer from './viewer/EmbeddedBrowserViewer';

interface BrowserLaneDetailsProps {
  lane: IBrowserLane;
  closing: boolean;
  displayMode?: BrowserDisplayMode | null;
  onClose: (lane: IBrowserLane) => void;
  onInventoryRefresh: () => Promise<void>;
}

const displayTime = (value?: number | null): string => {
  if (!value) return '—';
  const timestamp = value < 10_000_000_000 ? value * 1000 : value;
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString();
};

const formatBytes = (value?: number | null): string => {
  if (value == null || value < 0) return '—';
  const units = ['B', 'KiB', 'MiB', 'GiB'];
  let amount = value;
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit++;
  }
  return `${amount >= 10 || unit === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[unit]}`;
};

const Field: React.FC<{ label: string; children: React.ReactNode }> = ({ label, children }) => (
  <div className='min-w-0'>
    <div className='text-11px text-t-tertiary mb-3px'>{label}</div>
    <div className='text-12px text-t-primary break-words'>{children}</div>
  </div>
);

const valueOrDash = (value?: string | number | null): React.ReactNode =>
  value == null || value === '' ? '—' : value;

const formatDelay = (milliseconds?: number | null): string | null => {
  if (milliseconds == null || milliseconds < 0) return null;
  if (milliseconds < 1_000) return `${milliseconds} ms`;
  const seconds = milliseconds / 1_000;
  return `${Number.isInteger(seconds) ? seconds.toFixed(0) : seconds.toFixed(1)} s`;
};

const BrowserLaneDetails: React.FC<BrowserLaneDetailsProps> = ({
  lane,
  closing,
  displayMode,
  onClose,
  onInventoryRefresh,
}) => {
  const { t } = useTranslation();
  const activeTab =
    lane.tabs.find((tab) => tab.tab_id === lane.active_tab_id) ||
    lane.tabs.find((tab) => tab.active) ||
    lane.tabs[0];
  const owner = lane.owner;
  const identity = lane.identity;
  const queue = lane.queue;
  const retryDelay = formatDelay(queue?.retry_delay_ms);
  const recoveryAction = lane.recoverable
    ? t('browser.details.errorRetryAction')
    : t('browser.details.errorTerminalAction');
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
  const controlLabel = (state: string): string => {
    switch (state) {
      case 'agent':
        return t('browser.state.control.agent');
      case 'user':
        return t('browser.state.control.user');
      case 'idle':
        return t('browser.state.control.idle');
      default:
        return state;
    }
  };
  const identityLabel = (mode: string): string => {
    switch (mode) {
      case 'primary':
        return t('browser.state.identity.primary');
      case 'anonymous':
        return t('browser.state.identity.anonymous');
      case 'authenticated_replica':
        return t('browser.state.identity.authenticatedReplica');
      case 'isolated':
        return t('browser.state.identity.isolated');
      default:
        return mode;
    }
  };

  return (
    <div className='min-w-0 flex flex-col gap-12px'>
      <div className='flex items-start gap-10px'>
        <div className='min-w-0 flex-1'>
          <div className='flex flex-wrap items-center gap-6px'>
            <h2 className='m-0 text-18px leading-26px truncate'>
              {lane.title || activeTab?.title || lane.lane_name || t('browser.details.laneFallback')}
            </h2>
            <Tag color={lane.lifecycle_state === 'failed' ? 'red' : 'arcoblue'}>
              {lifecycleLabel(lane.lifecycle_state)}
            </Tag>
            <Tag color={lane.control_state === 'user' ? 'orange' : 'gray'}>
              {t('browser.details.controlState', {
                state: controlLabel(lane.control_state),
              })}
            </Tag>
          </div>
          <div className='mt-3px text-11px text-t-tertiary font-mono break-all'>{lane.lane_id}</div>
        </div>
        <Button
          status='danger'
          type='outline'
          loading={closing}
          icon={<Delete theme='outline' size='14' />}
          onClick={() => onClose(lane)}
        >
          {t('browser.details.closeLane')}
        </Button>
      </div>

      {lane.lifecycle_state === 'queued' && (
        <Alert
          type='warning'
          showIcon
          content={
            <span>
              {t('browser.details.waitingCapacity')}
              {queue?.position
                ? ` · ${t('browser.details.queuePositionInline', {
                    position: queue.position,
                  })}`
                : ''}
              {queue?.reason || queue?.reason_code
                ? ` · ${queue.reason || queue.reason_code}`
                : ''}
              {queue?.recommended_concurrency
                ? ` · ${t('browser.details.recommendedConcurrency', {
                    count: queue.recommended_concurrency,
                  })}`
                : ''}
              {retryDelay
                ? ` · ${t('browser.details.retryDelay', { delay: retryDelay })}`
                : ''}
              {queue?.owner_active != null || queue?.owner_queued != null
                ? ` · ${t('browser.details.ownerLoad', {
                    active: queue.owner_active ?? 0,
                    queued: queue.owner_queued ?? 0,
                  })}`
                : ''}
              {queue?.global_active != null || queue?.global_queued != null
                ? ` · ${t('browser.details.globalLoad', {
                    active: queue.global_active ?? 0,
                    queued: queue.global_queued ?? 0,
                  })}`
                : ''}
            </span>
          }
        />
      )}

      {lane.error_message && (
        <Alert
          type='error'
          showIcon
          content={
            <span>
              {lane.error_code ? `${lane.error_code}: ` : ''}
              {lane.error_message}
              {' · '}
              {lane.recoverable
                ? t('browser.details.retryable')
                : t('browser.details.notRetryable')}
              {' · '}
              {t('browser.details.nextAction', { action: recoveryAction })}
            </span>
          }
        />
      )}

      <EmbeddedBrowserViewer
        lane={lane}
        displayMode={displayMode}
        onInventoryRefresh={onInventoryRefresh}
      />

      <section className='border border-solid border-[var(--color-border-2)] rd-10px p-12px bg-bg-1'>
        <div className='text-13px font-600 mb-10px'>{t('browser.details.title')}</div>
        <div className='grid grid-cols-2 lg:grid-cols-3 gap-x-16px gap-y-12px'>
          <Field label={t('browser.details.fields.identity')}>
            {valueOrDash(identity?.label || (identity ? identityLabel(identity.mode) : undefined))}
            {identity?.mode === 'primary' && identity.shared_live !== false ? (
              <span className='ml-5px text-orange-6'>
                {t('browser.details.sharedLiveIdentity')}
              </span>
            ) : null}
          </Field>
          <Field label={t('browser.details.fields.identityGeneration')}>
            {identity?.mode === 'authenticated_replica'
              ? valueOrDash(identity.generation)
              : '—'}
          </Field>
          <Field label={t('browser.details.fields.lifecycle')}>
            {lifecycleLabel(lane.lifecycle_state)}
          </Field>
          <Field label={t('browser.details.fields.control')}>
            {controlLabel(lane.control_state)}
          </Field>
          <Field label={t('browser.details.fields.queuePosition')}>
            {valueOrDash(queue?.position)}
          </Field>
          <Field label={t('browser.details.fields.capacityReason')}>
            {valueOrDash(queue?.reason || queue?.reason_code)}
          </Field>
          <Field label={t('browser.details.fields.retryDelay')}>
            {valueOrDash(retryDelay)}
          </Field>
          <Field label={t('browser.details.fields.ownerCapacity')}>
            {queue?.owner_active != null || queue?.owner_queued != null
              ? t('browser.details.capacityCounts', {
                  active: queue.owner_active ?? 0,
                  queued: queue.owner_queued ?? 0,
                })
              : '—'}
          </Field>
          <Field label={t('browser.details.fields.globalCapacity')}>
            {queue?.global_active != null || queue?.global_queued != null
              ? t('browser.details.capacityCounts', {
                  active: queue.global_active ?? 0,
                  queued: queue.global_queued ?? 0,
                })
              : '—'}
          </Field>
          <Field label={t('browser.details.fields.resourceEstimate')}>
            {formatBytes(lane.resource_estimate_bytes)}
          </Field>
          <Field label={t('browser.details.fields.activePageTitle')}>
            {valueOrDash(activeTab?.title || lane.title)}
          </Field>
          <Field label={t('browser.details.fields.activeUrl')}>
            {valueOrDash(activeTab?.url || lane.url)}
          </Field>
          <Field label={t('browser.details.fields.lastActivity')}>
            {displayTime(lane.last_active_at)}
          </Field>
          <Field label={t('browser.details.fields.created')}>
            {displayTime(lane.created_at)}
          </Field>
          <Field label={t('browser.details.fields.owner')}>
            {valueOrDash(owner?.agent_name || owner?.label || owner?.surface)}
          </Field>
          <Field label={t('browser.details.fields.runtime')}>
            {valueOrDash(lane.runtime_label || lane.runtime_instance_id || owner?.runtime_instance_id)}
          </Field>
          <Field label={t('browser.details.fields.execution')}>
            {valueOrDash(lane.execution_id || owner?.execution_id)}
          </Field>
          <Field label={t('browser.details.fields.attempt')}>
            {valueOrDash(lane.attempt_id || owner?.attempt_id)}
          </Field>
          <Field label={t('browser.details.fields.clusterNode')}>
            {valueOrDash(lane.cluster_node_label || lane.cluster_node_id || owner?.cluster_node_id)}
          </Field>
          <Field label={t('browser.details.fields.tabs')}>{lane.tabs.length}</Field>
        </div>
      </section>
    </div>
  );
};

export default BrowserLaneDetails;
