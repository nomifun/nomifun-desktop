/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button } from '@arco-design/web-react';
import { Search } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import NomiInput from '@renderer/components/base/NomiInput';
import NomiSelect from '@renderer/components/base/NomiSelect';
import styles from './CreativeTemplateWorkspacePage.module.css';

export type TemplateKindFilter = 'all' | 'single-image' | 'multi-image-series';

interface TemplateWorkspaceToolbarProps {
  kind: TemplateKindFilter;
  category: string;
  query: string;
  categories: { value: string; label: string }[];
  count: number;
  total: number;
  disabled: boolean;
  onKindChange: (kind: TemplateKindFilter) => void;
  onCategoryChange: (category: string) => void;
  onQueryChange: (query: string) => void;
  onReset: () => void;
}

/** Compact text filters with inline counts and shared input/select controls. */
export default function TemplateWorkspaceToolbar(props: TemplateWorkspaceToolbarProps) {
  const { t } = useTranslation();
  const filters: { value: TemplateKindFilter; label: string }[] = [
    { value: 'all', label: t('creativeStudio.templates.workspace.allTemplates', { defaultValue: 'All templates' }) },
    { value: 'single-image', label: t('creativeStudio.templates.workspace.singleTemplates', { defaultValue: 'Single image' }) },
    { value: 'multi-image-series', label: t('creativeStudio.templates.workspace.seriesTemplates', { defaultValue: 'Image series' }) },
  ];
  const filtered = props.kind !== 'all' || props.category !== 'all' || Boolean(props.query.trim());

  return (
    <section className={styles.toolbar} aria-label={t('creativeStudio.templates.workspace.filters', { defaultValue: 'Filter templates' })}>
      <div className={styles.filterSummary}>
        <div className={`nomi-pill-tabs ${styles.kindFilters}`} role='group' aria-label={t('creativeStudio.templates.workspace.templateType', { defaultValue: 'Template type' })}>
          {filters.map((filter) => (
            <button key={filter.value} type='button' aria-pressed={props.kind === filter.value} disabled={props.disabled} onClick={() => props.onKindChange(filter.value)}>{filter.label}</button>
          ))}
        </div>
        <span className={styles.resultCount} role='status' aria-live='polite'>{t('creativeStudio.templates.workspace.resultCount', { visible: props.count, total: props.total, defaultValue: '{{visible}} of {{total}} templates' })}</span>
      </div>
      <div className={styles.searchFilters}>
        <NomiInput
          className={styles.search}
          size='small'
          prefix={<Search theme='outline' size={14} fill='currentColor' />}
          allowClear
          value={props.query}
          aria-label={t('creativeStudio.templates.workspace.searchPlaceholder', { defaultValue: 'Search names, categories, and descriptions' })}
          placeholder={t('creativeStudio.templates.workspace.searchPlaceholder', { defaultValue: 'Search names, categories, and descriptions' })}
          disabled={props.disabled}
          onChange={props.onQueryChange}
        />
        <NomiSelect
          className={styles.categorySelect}
          size='small'
          value={props.category}
          aria-label={t('creativeStudio.templates.workspace.allCategories', { defaultValue: 'All categories' })}
          options={[{ value: 'all', label: t('creativeStudio.templates.workspace.allCategories', { defaultValue: 'All categories' }) }, ...props.categories]}
          disabled={props.disabled}
          onChange={props.onCategoryChange}
        />
        {filtered ? <Button type='text' size='small' disabled={props.disabled} onClick={props.onReset}>{t('creativeStudio.templates.workspace.clearFilters', { defaultValue: 'Clear filters' })}</Button> : null}
      </div>
    </section>
  );
}
