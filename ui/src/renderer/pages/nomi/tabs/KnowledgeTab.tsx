/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Spin } from '@arco-design/web-react';
import KnowledgeControl from '@/renderer/pages/conversation/components/KnowledgeControl';
import { NomiSettingList, NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import type { useCompanion } from '../useNomi';

interface Props {
  companion: ReturnType<typeof useCompanion>;
}

/**
 * 伙伴「专属知识库」Tab —— 只挂该伙伴的私有知识库（KnowledgeControl kind:'companion'）。
 * 模型配置已迁出本 Tab（见 ChatTab 顶部，唯一事实源 = profile.model）。
 */
const KnowledgeTab: React.FC<Props> = ({ companion }) => {
  const { t } = useTranslation();
  const { profile } = companion;

  if (!profile) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  const companionName = profile.name;

  return (
    <div className='py-8px'>
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.settings.knowledge')}
          description={t('nomi.settings.knowledgeHint', { companionName })}
          controls={<KnowledgeControl target={{ kind: 'companion', id: profile.companion_id }} />}
        />
      </NomiSettingList>
    </div>
  );
};

export default KnowledgeTab;
