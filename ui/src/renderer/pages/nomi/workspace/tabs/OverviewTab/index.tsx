/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Spin } from '@arco-design/web-react';
import ContentAside from '@/renderer/components/layout/ContentAside';
import { customFigureMetaOf } from '@renderer/pages/companion/characters/customMeta';
import { useAsidePortal } from '../../AsideHost';
import type { WorkspaceTabProps } from '../../types';
import AppearanceSection from './AppearanceSection';
import FigurePanel from './FigurePanel';
import ModelsSection from './ModelsSection';
import PersonaSection from './PersonaSection';

/**
 * 总览 — the companion's identity and brains, in three titled sections:
 * 伙伴形象 (name / look / desktop visibility / growth), 伙伴设定 (persona + preset
 * reuse) and 模型配置 (chat model + a pointer to the app-level voice settings).
 *
 * Everything that used to be piled on here — the self-evolution disclosure
 * banner, the weekly digest card, the shared-store counters, the data-collection
 * alert — moved to the tab that owns it or was deleted outright.
 */
const OverviewTab: React.FC<WorkspaceTabProps> = ({ companionId, companion, onAttentionChange }) => {
  const { t } = useTranslation();
  const { profile, status, loading, patchCompanion, refresh } = companion;
  const [figurePaneOpen, setFigurePaneOpen] = useState(false);

  // A pane opened for one companion must not survive a switch to another.
  useEffect(() => {
    setFigurePaneOpen(false);
  }, [companionId]);

  // The one thing on this tab that can await the user: a missing chat model.
  const modelMissing = Boolean(status) && !status?.model_configured;
  useEffect(() => {
    onAttentionChange?.(modelMissing);
  }, [modelMissing, onAttentionChange]);

  const figure = customFigureMetaOf(profile);

  const aside = useAsidePortal(
    figurePaneOpen && profile ? (
      <ContentAside
        title={t('nomi.overview.changeFigure', { defaultValue: '更换形象' })}
        subtitle={t('nomi.settings.characterHint', {
          defaultValue: '选择常驻桌面的角色，悬停卡片可以预览它的兴奋状态',
        })}
        onClose={() => setFigurePaneOpen(false)}
        storageKey='nomifun:nomi-aside-overview'
        defaultWidth={400}
        minWidth={320}
      >
        <FigurePanel profile={profile} patchCompanion={patchCompanion} figure={figure} />
      </ContentAside>
    ) : null
  );

  const body =
    loading || !profile || !status ? (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    ) : (
      <div className='flex flex-col gap-16px'>
        <AppearanceSection
          profile={profile}
          status={status}
          patchCompanion={patchCompanion}
          figure={figure}
          figurePaneOpen={figurePaneOpen}
          onEditFigure={() => setFigurePaneOpen((open) => !open)}
        />
        <PersonaSection profile={profile} patchCompanion={patchCompanion} refresh={refresh} />
        <ModelsSection companion={companion} status={status} companionName={profile.name} />
      </div>
    );

  return (
    <>
      {body}
      {aside}
    </>
  );
};

export default OverviewTab;
