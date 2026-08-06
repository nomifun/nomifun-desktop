/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Tag } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import type {
  BrowserHostLifecycleState,
  BrowserIdentityMode,
  BrowserResourcePressureState,
  IBrowserHost,
  IBrowserOverview,
} from '@/common/browser/browserTypes';

interface BrowserHostDiagnosticsProps {
  overview: IBrowserOverview;
}

const formatBytes = (value?: number | null): string => {
  if (value == null || value < 0) return '-';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  let amount = value;
  let unit = 0;
  while (amount >= 1024 && unit < units.length - 1) {
    amount /= 1024;
    unit++;
  }
  return `${amount >= 10 || unit === 0 ? amount.toFixed(0) : amount.toFixed(1)} ${units[unit]}`;
};

const formatTime = (value?: number | null): string => {
  if (!value) return '-';
  const timestamp = value < 10_000_000_000 ? value * 1000 : value;
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? '-' : date.toLocaleString();
};

const valueOrDash = (value?: string | number | null): React.ReactNode =>
  value == null || value === '' ? '-' : value;

const hostStateColor = (state: BrowserHostLifecycleState): string => {
  if (state === 'running') return 'green';
  if (state === 'starting' || state === 'restarting') return 'orange';
  if (state === 'failed') return 'red';
  if (state === 'stopping') return 'arcoblue';
  return 'gray';
};

const pressureColor = (state?: BrowserResourcePressureState | null): string => {
  if (state === 'critical') return 'red';
  if (state === 'pressured') return 'orange';
  if (state === 'normal') return 'green';
  return 'gray';
};

const DiagnosticField: React.FC<{
  label: string;
  children: React.ReactNode;
  mono?: boolean;
}> = ({ label, children, mono = false }) => (
  <div className='min-w-0 rd-9px bg-[color:color-mix(in_srgb,var(--color-fill-1)_48%,transparent)] px-10px py-8px'>
    <div className='text-10px text-t-tertiary mb-3px'>{label}</div>
    <div className={mono ? 'text-11px text-t-primary font-mono break-all leading-17px' : 'text-12px text-t-primary break-words leading-18px'}>
      {children}
    </div>
  </div>
);

const BrowserHostDiagnostics: React.FC<BrowserHostDiagnosticsProps> = ({ overview }) => {
  const { t } = useTranslation();
  const hosts = overview.hosts ?? [];
  const capacity = overview.capacity;

  const hostStateLabel = (state: BrowserHostLifecycleState): string => {
    switch (state) {
      case 'stopped':
        return t('browser.diagnostics.hostState.stopped');
      case 'starting':
        return t('browser.state.lifecycle.starting');
      case 'running':
        return t('browser.state.lifecycle.running');
      case 'restarting':
        return t('browser.diagnostics.hostState.restarting');
      case 'stopping':
        return t('browser.state.lifecycle.stopping');
      case 'failed':
        return t('browser.state.lifecycle.failed');
      default:
        return state;
    }
  };

  const identityLabel = (mode?: BrowserIdentityMode | null): string => {
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
        return mode || '-';
    }
  };

  const pressureLabel = (state?: BrowserResourcePressureState | null): string => {
    switch (state) {
      case 'normal':
        return t('browser.state.pressure.normal');
      case 'pressured':
        return t('browser.state.pressure.pressured');
      case 'critical':
        return t('browser.state.pressure.critical');
      default:
        return state || '-';
    }
  };

  const renderHost = (host: IBrowserHost) => (
    <article
      key={host.host_id}
      className='min-w-0 rd-12px border border-solid border-[color:color-mix(in_srgb,var(--color-border-2)_58%,transparent)] bg-1 p-12px shadow-[0_6px_18px_rgba(15,23,42,0.03)]'
      data-browser-host-id={host.host_id}
    >
      <div className='flex items-center gap-6px mb-9px'>
        <span className='min-w-0 flex-1 truncate text-12px font-600'>
          {t('browser.diagnostics.host')}
        </span>
        <Tag size='small' color={hostStateColor(host.state)}>
          {hostStateLabel(host.state)}
        </Tag>
      </div>
      <div className='grid grid-cols-2 lg:grid-cols-3 gap-10px'>
        <DiagnosticField
          label={t('browser.diagnostics.fields.identity')}
        >
          {identityLabel(host.identity_mode)}
        </DiagnosticField>
        <DiagnosticField
          label={t('browser.diagnostics.fields.epoch')}
        >
          {valueOrDash(host.epoch)}
        </DiagnosticField>
        <DiagnosticField
          label={t('browser.diagnostics.fields.lanes')}
        >
          {valueOrDash(host.lane_count)}
        </DiagnosticField>
        <DiagnosticField
          label={t('browser.diagnostics.fields.visibility')}
        >
          {host.headful == null
            ? '-'
            : host.headful
              ? t('browser.displayMode.externalShort')
              : t('browser.displayMode.headlessShort')}
        </DiagnosticField>
        <DiagnosticField
          label={t('browser.diagnostics.fields.rss')}
        >
          {formatBytes(host.rss_bytes)}
        </DiagnosticField>
        <DiagnosticField
          label={t('browser.diagnostics.fields.hostId')}
          mono
        >
          {host.host_id}
        </DiagnosticField>
      </div>
    </article>
  );

  return (
    <details
      className='shrink-0 mb-14px overflow-hidden rd-14px border border-solid border-[color:color-mix(in_srgb,var(--color-border-2)_58%,transparent)] bg-fill-1 shadow-[0_8px_24px_rgba(15,23,42,0.035)]'
      data-browser-host-diagnostics
    >
      <summary className='cursor-pointer select-none px-14px py-11px text-12px text-t-primary transition-colors hover:bg-[color:color-mix(in_srgb,var(--color-fill-1)_48%,transparent)]'>
        <span className='font-600'>
          {t('browser.diagnostics.title')}
        </span>
        <span className='ml-8px text-t-tertiary'>
          {t('browser.diagnostics.hostCount', {
            count: hosts.length,
          })}
        </span>
        <Tag className='ml-8px' size='small' color={pressureColor(overview.pressure_state)}>
          {pressureLabel(overview.pressure_state)}
        </Tag>
      </summary>

      <div className='border-t border-solid border-[color:color-mix(in_srgb,var(--color-border-2)_52%,transparent)] border-b-0 border-l-0 border-r-0 p-12px'>
        <section className='mb-12px grid grid-cols-2 md:grid-cols-4 gap-10px'>
          <DiagnosticField label={t('browser.diagnostics.fields.activeLanes')}>
            {valueOrDash(capacity?.active_lanes ?? capacity?.active)}
            {capacity?.max_open_lanes != null ? ` / ${capacity.max_open_lanes}` : null}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.queuedRequests')}>
            {valueOrDash(capacity?.queued ?? overview.queued_lanes)}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.openLanes')}>
            {valueOrDash(overview.total_lanes)}
            {capacity?.max_open_lanes != null ? ` / ${capacity.max_open_lanes}` : null}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.managedHosts')}>
            {valueOrDash(overview.managed_host_count ?? hosts.length)}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.globalMemoryPressureThreshold')}>
            {formatBytes(capacity?.global_memory_pressure_threshold_bytes)}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.taskMemoryBudget')}>
            {formatBytes(capacity?.max_task_memory_bytes)}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.taskOperationLimit')}>
            {valueOrDash(capacity?.max_task_active_operations)}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.taskLaneLimit')}>
            {valueOrDash(capacity?.max_task_open_lanes)}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.taskTabLimit')}>
            {valueOrDash(capacity?.max_task_tabs)}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.pendingCleanup')}>
            {valueOrDash(overview.pending_cleanup_count)}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.recommendedConcurrency')}>
            {valueOrDash(capacity?.recommended_concurrency)}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.capacityReason')}>
            {valueOrDash(capacity?.reason_code)}
          </DiagnosticField>
          <DiagnosticField label={t('browser.diagnostics.fields.updated')}>
            {formatTime(overview.updated_at)}
          </DiagnosticField>
        </section>

        {hosts.length > 0 ? (
          <section className='grid grid-cols-1 xl:grid-cols-2 gap-10px'>
            {hosts.map(renderHost)}
          </section>
        ) : (
          <div className='text-12px text-t-tertiary py-6px'>
            {t('browser.diagnostics.noHosts')}
          </div>
        )}
      </div>
    </details>
  );
};

export default BrowserHostDiagnostics;
