/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Spin } from '@arco-design/web-react';
import React, { useMemo } from 'react';

import NomiSelect from '@/renderer/components/base/NomiSelect';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';

import {
  buildCreativeModelGroups,
  creativeModelSelectorState,
  creativeModelTaskFor,
  findCreativeModelOption,
  flattenCreativeModelGroups,
} from './catalog';
import styles from './CreativeModelSelect.module.css';
import type {
  CreativeModelCatalogSnapshot,
  CreativeModelFilter,
  CreativeModelOption,
  CreativeModelSelectCopy,
  CreativeModelSelectionRef,
  CreativeModelSelectorState,
} from './types';

export interface CreativeModelSelectProps {
  catalog: CreativeModelCatalogSnapshot;
  filter: CreativeModelFilter;
  value: CreativeModelSelectionRef | null;
  onChange: (selection: CreativeModelOption) => void;
  disabled?: boolean;
  label?: string;
  copy?: Partial<CreativeModelSelectCopy>;
  onOpenModelSettings?: () => void;
  className?: string;
}

const DEFAULT_COPY: CreativeModelSelectCopy = {
  label: '生成模型',
  placeholder: '选择兼容模型',
  loading: '正在加载模型目录…',
  noProvider: '尚未配置模型服务商。',
  noCompatibleModel: '没有支持当前任务的已启用模型。',
  disabled: '当前步骤暂不可更改模型。',
  error: '模型目录加载失败。',
  unavailable: '已选择的模型当前不可用，请重新选择。',
  retry: '重试',
  configureModels: '管理模型',
};

const optionKey = (value: CreativeModelSelectionRef): string =>
  JSON.stringify([value.providerId, value.model]);

const stateCopy = (
  state: Exclude<CreativeModelSelectorState, 'ready'>,
  copy: CreativeModelSelectCopy
): string => {
  switch (state) {
    case 'loading':
      return copy.loading;
    case 'no-provider':
      return copy.noProvider;
    case 'no-compatible-model':
      return copy.noCompatibleModel;
    case 'disabled':
      return copy.disabled;
    case 'error':
      return copy.error;
  }
};

const stateTone = (
  state: Exclude<CreativeModelSelectorState, 'ready'>
): 'neutral' | 'danger' | 'warning' => {
  if (state === 'error') return 'danger';
  if (state === 'no-compatible-model') return 'warning';
  return 'neutral';
};

/**
 * Controlled Creative Studio model picker. Catalog fetching is deliberately
 * outside this component: callers can pass the NomiFun adapter or a stable
 * test/story snapshot without changing selection semantics.
 */
const CreativeModelSelect: React.FC<CreativeModelSelectProps> = ({
  catalog,
  filter,
  value,
  onChange,
  disabled = false,
  label,
  copy: copyOverride,
  onOpenModelSettings,
  className,
}) => {
  const providerLabel = useModelSelectorProviderLabel();
  const copy = { ...DEFAULT_COPY, ...copyOverride };
  const task = creativeModelTaskFor(filter);
  const groups = useMemo(
    () => buildCreativeModelGroups(catalog.providers, filter, providerLabel),
    [catalog.providers, filter, providerLabel]
  );
  const options = useMemo(() => flattenCreativeModelGroups(groups), [groups]);
  const optionByKey = useMemo(
    () => new Map(options.map((option) => [optionKey(option), option])),
    [options]
  );
  const selected = findCreativeModelOption(groups, value);
  const state = creativeModelSelectorState({ catalog, groups, disabled });
  const selectedUnavailable = value !== null && selected === null && catalog.status === 'ready';
  const rootClassName = className ? `${styles.root} ${className}` : styles.root;

  const status = state === 'ready' ? null : state;
  const canConfigure =
    onOpenModelSettings !== undefined &&
    (state === 'no-provider' || state === 'no-compatible-model');

  return (
    <div
      className={rootClassName}
      data-creative-model-select
      data-state={state}
      data-selection-state={selectedUnavailable ? 'unavailable' : selected ? 'resolved' : 'empty'}
    >
      <div className={styles.labelRow}>
        <span className={styles.label}>{label ?? copy.label}</span>
        <span className={styles.task}>{task}</span>
      </div>
      <NomiSelect
        className={styles.select}
        showSearch
        disabled={state !== 'ready'}
        placeholder={copy.placeholder}
        value={value ? optionKey(value) : undefined}
        aria-label={label ?? copy.label}
        onChange={(key: string) => {
          const next = optionByKey.get(key);
          if (next) onChange(next);
        }}
      >
        {selectedUnavailable && value && (
          <NomiSelect.Option value={optionKey(value)} disabled>
            {value.model} · {copy.unavailable}
          </NomiSelect.Option>
        )}
        {groups.map((group) => (
          <NomiSelect.OptGroup key={group.providerId} label={group.providerName}>
            {group.models.map((option) => (
              <NomiSelect.Option key={optionKey(option)} value={optionKey(option)}>
                <span className={styles.option}>
                  <span className={styles.optionModel}>{option.model}</span>
                  <span className={styles.optionProtocol}>{option.protocol}</span>
                </span>
              </NomiSelect.Option>
            ))}
          </NomiSelect.OptGroup>
        ))}
      </NomiSelect>

      {selected && (
        <div className={styles.selectionMeta} aria-label={`${selected.providerName} · ${selected.protocol}`}>
          <span>{selected.providerName}</span>
          <span className={styles.separator} aria-hidden='true'>
            ·
          </span>
          <span>{selected.protocol}</span>
        </div>
      )}

      {selectedUnavailable && state === 'ready' && (
        <div className={styles.status} data-tone='warning' role='status'>
          <span className={styles.statusText}>{copy.unavailable}</span>
        </div>
      )}

      {status && (
        <div
          className={styles.status}
          data-tone={stateTone(status)}
          role={status === 'error' ? 'alert' : 'status'}
          aria-live='polite'
        >
          {status === 'loading' && <Spin size={12} />}
          <span className={styles.statusText}>
            {stateCopy(status, copy)}
            {status === 'error' && catalog.error?.message ? ` ${catalog.error.message}` : ''}
          </span>
          {status === 'error' && catalog.refresh && (
            <button type='button' className={styles.action} onClick={catalog.refresh}>
              {copy.retry}
            </button>
          )}
          {canConfigure && (
            <button type='button' className={styles.action} onClick={onOpenModelSettings}>
              {copy.configureModels}
            </button>
          )}
        </div>
      )}
    </div>
  );
};

export default CreativeModelSelect;
