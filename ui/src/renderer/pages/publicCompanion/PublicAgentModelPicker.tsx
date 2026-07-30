/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import type { IPublicAgentModel } from '@/common/adapter/ipcBridge';
import TaskModelSelect from '@/renderer/components/agent/TaskModelSelect';

interface Props {
  value: IPublicAgentModel;
  /** Emits the FULL model ref on any change. */
  onChange: (model: IPublicAgentModel) => void;
}

/**
 * 对外伙伴回答陌生人所用模型的配置控件 —— 统一 TaskModelSelect（task='chat'）。
 * 模型清单来自后端 catalog resolve；一次点选即产出完整 provider+model 组合，
 * 不会遗留跨 provider 的无效组合。
 */
const PublicAgentModelPicker: React.FC<Props> = ({ value, onChange }) => {
  const { t } = useTranslation();

  return (
    <TaskModelSelect
      task='chat'
      value={value.provider_id && value.model ? { providerId: value.provider_id, model: value.model } : null}
      onSelect={(selection) => onChange({ provider_id: selection.providerId, model: selection.model })}
      placeholder={t('publicCompanion.identity.modelName', { defaultValue: '选择模型' })}
    />
  );
};

export default PublicAgentModelPicker;
