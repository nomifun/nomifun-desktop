/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Alert, Button, Tag } from '@arco-design/web-react';
import { BringToFront, Delete, WebPage } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import type { IBrowserLane } from '@/common/browser/browserTypes';

interface BrowserLaneDetailsProps {
  lane: IBrowserLane;
  closing: boolean;
  visibilityChanging: boolean;
  actionsDisabled?: boolean;
  hostHeadful?: boolean | null;
  canChangeVisibility: boolean;
  onClose: (lane: IBrowserLane) => void;
  onForeground: (lane: IBrowserLane) => void;
  onBackground: (lane: IBrowserLane) => void;
}

const DASH = '—';

const displayTime = (value?: number | null): string => {
  if (!value) return DASH;
  const timestamp = value < 10_000_000_000 ? value * 1000 : value;
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? DASH : date.toLocaleString();
};

const formatBytes = (value?: number | null): string => {
  if (value == null || value < 0) return DASH;
  const units = ['B', 'KiB', 'MiB', 'GiB'];
  let amount = value;
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit++;
  }
  return `${amount >= 10 || unit === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[unit]}`;
};

const formatDelay = (milliseconds?: number | null): string | null => {
  if (milliseconds == null || milliseconds < 0) return null;
  if (milliseconds < 1_000) return `${milliseconds} ms`;
  const seconds = milliseconds / 1_000;
  return `${Number.isInteger(seconds) ? seconds.toFixed(0) : seconds.toFixed(1)} s`;
};

const valueOrDash = (value?: string | number | null): React.ReactNode =>
  value == null || value === '' ? DASH : value;

const lifecycleColor = (state: string): string => {
  if (state === 'running') return 'green';
  if (state === 'queued' || state === 'starting' || state === 'stopping') return 'orange';
  if (state === 'failed') return 'red';
  if (state === 'frozen') return 'arcoblue';
  return 'gray';
};

/**
 * A recoverable managed-browser restart is expected lifecycle churn (the Hub
 * relaunches the Host), not a terminal failure: present it as information so
 * users are not pushed to close a healthy lane. Every other error stays red.
 */
const isInformationalLaneError = (lane: IBrowserLane): boolean =>
  lane.error_code === 'browser_restarted' && lane.recoverable === true;

const Field: React.FC<{
  label: string;
  children: React.ReactNode;
  wide?: boolean;
  mono?: boolean;
}> = ({ label, children, wide = false, mono = false }) => (
  <div
    className={`${wide ? 'sm:col-span-2' : ''} min-w-0 rd-10px border border-solid border-[color:color-mix(in_srgb,var(--color-border-2)_56%,transparent)] bg-[color:color-mix(in_srgb,var(--color-fill-1)_44%,transparent)] px-11px py-9px`}
  >
    <div className='mb-4px text-10px font-500 uppercase tracking-[0.04em] text-t-tertiary'>
      {label}
    </div>
    <div
      className={
        mono
          ? 'break-all font-mono text-11px leading-18px text-t-primary'
          : 'break-words text-12px leading-18px text-t-primary'
      }
    >
      {children}
    </div>
  </div>
);

const StatusSection: React.FC<{
  id: string;
  title: string;
  description: string;
  children: React.ReactNode;
}> = ({ id, title, description, children }) => (
  <section
    className='min-w-0 rd-14px border border-solid border-[color:color-mix(in_srgb,var(--color-border-2)_74%,transparent)] bg-1 p-15px shadow-[0_6px_18px_rgba(15,23,42,0.025)]'
    data-browser-status-section={id}
  >
    <div className='mb-11px border-b border-solid border-[color:color-mix(in_srgb,var(--color-border-2)_52%,transparent)] border-l-0 border-r-0 border-t-0 pb-10px'>
      <h3 className='m-0 text-13px font-600 leading-20px'>{title}</h3>
      <div className='mt-2px text-11px leading-17px text-t-tertiary'>{description}</div>
    </div>
    <div className='grid grid-cols-1 gap-9px sm:grid-cols-2'>{children}</div>
  </section>
);

const BrowserLaneDetails: React.FC<BrowserLaneDetailsProps> = ({
  lane,
  closing,
  visibilityChanging,
  actionsDisabled = false,
  hostHeadful,
  canChangeVisibility,
  onClose,
  onForeground,
  onBackground,
}) => {
  const { t } = useTranslation();
  const activeTab =
    lane.tabs.find((tab) => tab.tab_id === lane.active_tab_id) ||
    lane.tabs.find((tab) => tab.active) ||
    lane.tabs[0];
  const activeTitle = activeTab?.title || lane.title;
  const activeUrl = activeTab?.url || lane.url;
  const owner = lane.owner;
  const identity = lane.identity;
  const queue = lane.queue;
  const retryDelay = formatDelay(queue?.retry_delay_ms);
  const isQueued = lane.lifecycle_state === 'queued' || queue?.position != null;
  const activeOperations =
    lane.active_operation_count != null
      ? lane.active_operation_count
      : lane.active_operation === true
        ? 1
        : lane.active_operation === false
          ? 0
          : null;

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
      case 'unknown':
        return t('browser.state.lifecycle.unknown');
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
    <div
      className='min-w-0 flex flex-col gap-12px'
      data-browser-lane-management
      data-browser-lane-id={lane.lane_id}
    >
      <header className='rd-14px border border-solid border-[color:color-mix(in_srgb,var(--color-border-2)_82%,transparent)] bg-1 px-16px py-15px shadow-[0_7px_22px_rgba(15,23,42,0.03)]'>
        <div className='flex flex-wrap items-start gap-12px'>
          <div className='size-36px shrink-0 rd-10px border border-solid border-[rgba(var(--primary-6),0.16)] bg-primary-1 text-primary-6 flex items-center justify-center'>
            <WebPage theme='outline' size='18' />
          </div>
          <div className='min-w-0 flex-1'>
            <div className='flex flex-wrap items-center gap-6px'>
              <h2 className='m-0 min-w-0 truncate text-18px leading-26px'>
                {lane.lane_name || activeTitle || t('browser.details.laneFallback')}
              </h2>
              <Tag color={lifecycleColor(lane.lifecycle_state)}>
                {lifecycleLabel(lane.lifecycle_state)}
              </Tag>
              <Tag color='gray'>{t('browser.details.managed')}</Tag>
            </div>
            <div className='mt-4px text-12px font-500 leading-18px text-t-primary'>
              {valueOrDash(activeTitle)}
            </div>
            <div className='mt-2px break-all font-mono text-11px leading-17px text-t-secondary'>
              {valueOrDash(activeUrl)}
            </div>
            <div className='mt-7px text-11px leading-17px text-t-tertiary'>
              {t('browser.details.backgroundExecution')}
            </div>
            <div className='mt-4px break-all font-mono text-10px leading-16px text-t-tertiary'>
              {lane.lane_id}
            </div>
          </div>
          <div className='flex flex-wrap items-center justify-end gap-8px'>
            {canChangeVisibility && hostHeadful != null && (
              <Button
                type='primary'
                loading={visibilityChanging}
                disabled={closing || actionsDisabled}
                icon={<BringToFront theme='outline' size='14' />}
                onClick={() =>
                  hostHeadful ? onBackground(lane) : onForeground(lane)
                }
                data-browser-visibility-action={hostHeadful ? 'background' : 'foreground'}
              >
                {hostHeadful
                  ? t('browser.background.action')
                  : t('browser.foreground.action')}
              </Button>
            )}
            <Button
              status='danger'
              type='outline'
              loading={closing}
              disabled={visibilityChanging || actionsDisabled}
              icon={<Delete theme='outline' size='14' />}
              onClick={() => onClose(lane)}
            >
              {t('browser.details.closeLane')}
            </Button>
          </div>
        </div>
      </header>

      {isQueued && (
        <Alert
          type='warning'
          showIcon
          content={
            <span>
              {t('browser.details.waitingCapacity')}
              {queue?.position != null
                ? ` · ${t('browser.details.queuePositionInline', {
                    position: queue.position,
                  })}`
                : ''}
              {queue?.reason || queue?.reason_code
                ? ` · ${queue.reason || queue.reason_code}`
                : ''}
              {queue?.recommended_concurrency != null
                ? ` · ${t('browser.details.recommendedConcurrency', {
                    count: queue.recommended_concurrency,
                  })}`
                : ''}
              {retryDelay
                ? ` · ${t('browser.details.retryDelay', { delay: retryDelay })}`
                : ''}
            </span>
          }
        />
      )}

      {(lane.error_code || lane.error_message) &&
        (isInformationalLaneError(lane) ? (
          <Alert
            type='info'
            showIcon
            content={
              <div>
                <div className='font-500'>
                  {lane.error_code ? `${lane.error_code}: ` : ''}
                  {lane.error_message || DASH}
                </div>
                <div className='mt-2px text-11px'>
                  {t('browser.details.restartedNotice')}
                </div>
              </div>
            }
          />
        ) : (
          <Alert
            type='error'
            showIcon
            content={
              <div>
                <div className='font-500'>
                  {lane.error_code ? `${lane.error_code}: ` : ''}
                  {lane.error_message || DASH}
                </div>
                {lane.recoverable != null && (
                  <div className='mt-2px text-11px'>
                    {lane.recoverable
                      ? t('browser.details.errorRecoverable')
                      : t('browser.details.errorTerminal')}
                  </div>
                )}
              </div>
            }
          />
        ))}

      <StatusSection
        id='current'
        title={t('browser.details.sections.current')}
        description={t('browser.details.sections.currentDescription')}
      >
        <Field label={t('browser.details.fields.lifecycle')}>
          <Tag size='small' color={lifecycleColor(lane.lifecycle_state)}>
            {lifecycleLabel(lane.lifecycle_state)}
          </Tag>
        </Field>
        <Field label={t('browser.details.fields.activeOperations')}>
          {valueOrDash(activeOperations)}
        </Field>
        <Field label={t('browser.details.fields.activePageTitle')} wide>
          {valueOrDash(activeTitle)}
        </Field>
        <Field label={t('browser.details.fields.activeUrl')} wide mono>
          {valueOrDash(activeUrl)}
        </Field>
        <Field label={t('browser.details.fields.tabs')}>{lane.tabs.length}</Field>
        <Field label={t('browser.details.fields.resourceEstimate')}>
          {formatBytes(lane.resource_estimate_bytes)}
        </Field>
        <Field label={t('browser.details.fields.lastActivity')}>
          {displayTime(lane.last_active_at)}
        </Field>
        <Field label={t('browser.details.fields.created')}>
          {displayTime(lane.created_at)}
        </Field>
      </StatusSection>

      <div className='grid grid-cols-1 gap-12px xl:grid-cols-2'>
        <StatusSection
          id='identity-owner'
          title={t('browser.details.sections.identityOwner')}
          description={t('browser.details.sections.identityOwnerDescription')}
        >
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
              : DASH}
          </Field>
          <Field label={t('browser.details.fields.owner')} wide>
            {valueOrDash(owner?.agent_name || owner?.label || owner?.surface)}
          </Field>
          <Field label={t('browser.details.fields.runtime')} mono>
            {valueOrDash(lane.runtime_label || lane.runtime_instance_id || owner?.runtime_instance_id)}
          </Field>
          <Field label={t('browser.details.fields.execution')} mono>
            {valueOrDash(lane.execution_id || owner?.execution_id)}
          </Field>
          <Field label={t('browser.details.fields.attempt')} mono>
            {valueOrDash(lane.attempt_id || owner?.attempt_id)}
          </Field>
          <Field label={t('browser.details.fields.clusterNode')} mono>
            {valueOrDash(lane.cluster_node_label || lane.cluster_node_id || owner?.cluster_node_id)}
          </Field>
        </StatusSection>

        <StatusSection
          id='queue-resources'
          title={t('browser.details.sections.queueResources')}
          description={t('browser.details.sections.queueResourcesDescription')}
        >
          <Field label={t('browser.details.fields.queueState')}>
            {isQueued
              ? t('browser.details.queueState.queued')
              : t('browser.details.queueState.notQueued')}
          </Field>
          <Field label={t('browser.details.fields.queuePosition')}>
            {valueOrDash(queue?.position)}
          </Field>
          <Field label={t('browser.details.fields.capacityReason')} wide>
            {valueOrDash(queue?.reason || queue?.reason_code)}
          </Field>
          <Field label={t('browser.details.fields.retryDelay')}>
            {valueOrDash(retryDelay)}
          </Field>
          <Field label={t('browser.details.fields.recommendedConcurrency')}>
            {valueOrDash(queue?.recommended_concurrency)}
          </Field>
          <Field label={t('browser.details.fields.ownerCapacity')}>
            {queue?.owner_active != null || queue?.owner_queued != null
              ? t('browser.details.capacityCounts', {
                  active: queue.owner_active ?? 0,
                  queued: queue.owner_queued ?? 0,
                })
              : DASH}
          </Field>
          <Field label={t('browser.details.fields.globalCapacity')}>
            {queue?.global_active != null || queue?.global_queued != null
              ? t('browser.details.capacityCounts', {
                  active: queue.global_active ?? 0,
                  queued: queue.global_queued ?? 0,
                })
              : DASH}
          </Field>
          <Field label={t('browser.details.fields.resourceEstimate')} wide>
            {formatBytes(lane.resource_estimate_bytes)}
          </Field>
        </StatusSection>
      </div>
    </div>
  );
};

export default BrowserLaneDetails;
