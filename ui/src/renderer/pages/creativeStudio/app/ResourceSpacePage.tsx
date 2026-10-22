/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button } from '@arco-design/web-react';
import { History, ImageFiles, PageTemplate } from '@icon-park/react';
import React, { Suspense, useCallback, useEffect, useId, useState } from 'react';
import { useTranslation } from 'react-i18next';

import PageHeader from '@/renderer/components/layout/PageHeader';
import RouteErrorBoundary from '@/renderer/components/layout/RouteErrorBoundary';
import { ResourceSpaceChromeProvider } from './ResourceSpaceChrome';
import styles from './ResourceSpacePage.module.css';

export type ResourceSpaceSection = 'assets' | 'templates';

interface ResourceSpacePageProps {
  section: ResourceSpaceSection;
  onSectionChange: (section: ResourceSpaceSection) => void;
  assets: React.ReactNode;
  templates: React.ReactNode;
}

/** Shared resource chrome; visited panels retain their own controls and state. */
export default function ResourceSpacePage({ section, onSectionChange, assets, templates }: ResourceSpacePageProps) {
  const { t } = useTranslation();
  const id = useId();
  const [visited, setVisited] = useState({ assets: section === 'assets', templates: section === 'templates' });
  const [assetActions, setAssetActions] = useState<HTMLDivElement | null>(null);
  const [templateActions, setTemplateActions] = useState<HTMLDivElement | null>(null);
  const [taskActions, setTaskActions] = useState<HTMLDivElement | null>(null);
  const [tasksMounted, setTasksMounted] = useState(false);
  const [tasksLoadFailed, setTasksLoadFailed] = useState(false);
  const [openTaskCenterOnMount, setOpenTaskCenterOnMount] = useState(false);
  const handleTaskActionsMount = useCallback((mounted: boolean) => {
    setTasksMounted(mounted);
    if (mounted) setTasksLoadFailed(false);
  }, []);

  useEffect(() => {
    setVisited((current) => current[section] ? current : { ...current, [section]: true });
  }, [section]);

  const sections = [
    { value: 'assets', label: t('creativeStudio.resources.materials'), icon: <ImageFiles theme='outline' size={16} fill='currentColor' /> },
    { value: 'templates', label: t('creativeStudio.resources.templates'), icon: <PageTemplate theme='outline' size={16} fill='currentColor' /> },
  ] as const;

  const onTabKeyDown = (event: React.KeyboardEvent<HTMLButtonElement>, index: number) => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? sections.length - 1
      : (index + (event.key === 'ArrowRight' ? 1 : sections.length - 1)) % sections.length;
    onSectionChange(sections[next].value);
    document.getElementById(`${id}-tab-${sections[next].value}`)?.focus();
  };

  const activateTasks = () => {
    // Mounting the existing template runtime may resume its unfinished runs.
    // Only do so after the user opens templates or explicitly requests tasks.
    setOpenTaskCenterOnMount(true);
    setVisited((current) => ({ ...current, templates: true }));
  };

  return <div className={styles.page} data-resource-space data-resource-space-section={section}>
    <div className={styles.header}>
      <PageHeader
        title={t('creativeStudio.resources.title')}
        description={t('creativeStudio.resources.description')}
        actions={<>
          <div ref={setAssetActions} className={styles.headerActions} hidden={section !== 'assets'} data-resource-header-actions='assets' />
          <div ref={setTemplateActions} className={styles.headerActions} hidden={section !== 'templates'} data-resource-header-actions='templates' />
        </>}
      />
    </div>
    <div className={styles.tabRow}>
      <div className={styles.tabs} role='tablist' aria-label={t('creativeStudio.resources.tabsLabel')}>
        {sections.map((item, index) => <button
          key={item.value}
          id={`${id}-tab-${item.value}`}
          type='button'
          role='tab'
          aria-selected={section === item.value}
          aria-controls={`${id}-panel-${item.value}`}
          tabIndex={section === item.value ? 0 : -1}
          className={styles.tab}
          onClick={() => onSectionChange(item.value)}
          onKeyDown={(event) => onTabKeyDown(event, index)}
        >{item.icon}{item.label}</button>)}
      </div>
      <div ref={setTaskActions} className={styles.taskActions} data-resource-template-tasks>
        {!tasksMounted ? <Button
          type='text'
          size='small'
          status={tasksLoadFailed ? 'danger' : 'default'}
          icon={<History theme='outline' size={16} fill='currentColor' />}
          loading={!tasksLoadFailed && (visited.templates || section === 'templates')}
          onClick={tasksLoadFailed ? () => onSectionChange('templates') : activateTasks}
        >{t(tasksLoadFailed ? 'creativeStudio.resources.tasksLoadFailed' : 'creativeStudio.resources.templateTasks')}</Button> : null}
      </div>
    </div>
    <div className={styles.panels}>
      {sections.map(({ value }) => <section
        key={value}
        id={`${id}-panel-${value}`}
        className={styles.panel}
        role='tabpanel'
        aria-labelledby={`${id}-tab-${value}`}
        hidden={section !== value}
      >{visited[value] || section === value ? <ResourceSpaceChromeProvider value={{
        section: value,
        headerActions: value === 'assets' ? assetActions : templateActions,
        taskActions,
        openTaskCenterOnMount,
        onTaskActionsMount: handleTaskActionsMount,
      }}>
        <RouteErrorBoundary resetKey={`${value}-${section}`} onError={value === 'templates' ? () => setTasksLoadFailed(true) : undefined}>
          <Suspense fallback={<div className={styles.loading} role='status'>{t('creativeStudio.resources.loading')}</div>}>
            {value === 'assets' ? assets : templates}
          </Suspense>
        </RouteErrorBoundary>
      </ResourceSpaceChromeProvider> : null}</section>)}
    </div>
  </div>;
}
