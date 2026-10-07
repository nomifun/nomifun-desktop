/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { lazy, Suspense } from 'react';
import CreativePromptPicker from '@/renderer/pages/creativeStudio/components/CreativePromptPicker';
import CreativeResourceDialog from '@/renderer/pages/creativeStudio/components/CreativeResourceDialog';
import type { PromptLibraryItem } from '@/renderer/pages/creativeStudio/prompts';
import styles from './CreationControls.module.css';

const CreativeTemplateRoute = lazy(() => import('@/renderer/pages/creativeStudio/templates/page/CreativeTemplateRoute'));

interface CreationResourceDialogProps {
  view: 'prompts' | 'templates';
  locale: string;
  selectedPromptId: string | null;
  onPromptSelect(item: PromptLibraryItem): void;
  onClose(): void;
}

/** Composer resources stay in context instead of navigating away from the draft. */
export default function CreationResourceDialog({
  view,
  locale,
  selectedPromptId,
  onPromptSelect,
  onClose,
}: CreationResourceDialogProps) {
  return (
    <CreativeResourceDialog
      kind={view}
      title={view === 'prompts' ? '选择提示词' : '模板工作台'}
      scope='conversation'
      onClose={onClose}
    >
      {view === 'prompts' ? (
        <CreativePromptPicker
          locale={locale}
          selectedId={selectedPromptId}
          applyLabel='使用提示词'
          onSelect={onPromptSelect}
        />
      ) : (
        <Suspense
          fallback={<div className={styles.resourceLoading}>正在加载模板工作台…</div>}
        >
          <CreativeTemplateRoute />
        </Suspense>
      )}
    </CreativeResourceDialog>
  );
}
