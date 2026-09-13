import { agentPlatform } from '@/common/adapter/ipcBridge';
import { ipcBridge } from '@/common';
import type { TProviderWithModel } from '@/common/config/storage';
import type {
  AgentPresetEditorResponse,
  OfficialPresetKey,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import { parseAgentPresetId, parseCompanionId } from '@/common/types/ids';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import { Message, Select, Spin } from '@arco-design/web-react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';

type Props = {
  targetKind: 'companion' | 'robot' | 'customer' | 'creative_studio_canvas';
  targetId: string;
  defaultTemplateKey: OfficialPresetKey;
  model?: Pick<TProviderWithModel, 'id' | 'use_model'>;
  conversationId?: string | null;
  disabled?: boolean;
  onChanged?: () => void;
};

const templateValue = (key: OfficialPresetKey): string => `template:${key}`;
const presetValue = (id: string): string => `preset:${id}`;
const sorted = (values: Iterable<string>): string[] => [...values].sort();

export const templateForEditor = (
  editor: AgentPresetEditorResponse,
  templates: readonly OfficialPresetTemplate[]
): OfficialPresetTemplate | undefined => {
  const document = editor.revision?.document;
  if (!document) return undefined;
  const capabilities = JSON.stringify(sorted(document.enabled_capabilities.map((item) => item.capability.id)));
  const skills = JSON.stringify(sorted(document.skill_bindings.map((item) => item.id)));
  return templates.find((template) =>
    JSON.stringify(sorted(template.seed.enabled_capabilities.map((item) => item.id))) === capabilities
      && JSON.stringify(sorted(template.seed.skill_bindings.map((item) => item.id))) === skills
  );
};

const ProductAgentBindingSelect: React.FC<Props> = ({
  targetKind,
  targetId,
  defaultTemplateKey,
  model,
  conversationId,
  disabled = false,
  onChanged,
}) => {
  const { t } = useTranslation();
  const { library, presets, isLoading, error } = useAgentPresets();
  const [value, setValue] = useState(templateValue(defaultTemplateKey));
  const [loadingBinding, setLoadingBinding] = useState(true);
  const [saving, setSaving] = useState(false);
  const templates = useMemo(() => library?.official_templates ?? [], [library]);
  const presetSignature = presets.map((preset) => preset.preset_id).join('\u0000');
  const templateSignature = templates.map((template) => template.template_key).join('\u0000');

  const templateName = useCallback((key: OfficialPresetKey): string =>
    t(`agentSettings.template.${TEMPLATE_I18N_PATH[key]}.name`), [t]);

  useEffect(() => {
    let cancelled = false;
    if (!model) {
      setValue(templateValue(defaultTemplateKey));
      setLoadingBinding(false);
      return () => { cancelled = true; };
    }
    setLoadingBinding(true);
    void agentPlatform.getBinding.invoke({ target_kind: targetKind, target_id: targetId })
      .then(async (record) => {
        if (!record) return templateValue(defaultTemplateKey);
        const direct = presets.find((preset) =>
          preset.preset_id === record.agent_binding.preset_revision_ref.preset_id
        );
        if (direct) return presetValue(direct.preset_id);
        const editor = await agentPlatform.getEditor.invoke({
          preset_id: record.agent_binding.preset_revision_ref.preset_id,
          revision: record.agent_binding.preset_revision_ref.revision,
        });
        const template = templateForEditor(editor, templates);
        return template ? templateValue(template.template_key) : presetValue(editor.preset.preset_id);
      })
      .then((next) => { if (!cancelled) setValue(next); })
      .catch((reason) => {
        console.error('[ProductAgentBindingSelect] Failed to load binding:', reason);
      })
      .finally(() => { if (!cancelled) setLoadingBinding(false); });
    return () => { cancelled = true; };
  }, [defaultTemplateKey, model?.id, model?.use_model, presetSignature, targetId, targetKind, templateSignature]);

  const options = useMemo(() => [
    ...templates.map((template) => ({
      value: templateValue(template.template_key),
      label: `${templateName(template.template_key)} · ${t('agentSettings.template.readOnly')}`,
    })),
    ...presets.filter((preset) => preset.current_stable_revision).map((preset) => ({
      value: presetValue(preset.preset_id),
      label: preset.display_name,
    })),
  ], [presets, t, templateName, templates]);

  const change = useCallback(async (next: string) => {
    if (saving || next === value) return;
    setSaving(true);
    try {
      let presetId: string;
      if (next.startsWith('template:')) {
        const templateKey = next.slice('template:'.length) as OfficialPresetKey;
        const editor = await agentPlatform.createFromTemplate.invoke({
          template_id: templateKey,
          request: {
            display_name: templateKey,
            model_route_refs: {},
            chat_route_records: {},
            reuse_existing: true,
            ...(model ? { model: { provider_id: model.id, model: model.use_model } } : {}),
          },
        });
        presetId = editor.preset.preset_id;
      } else {
        presetId = next.slice('preset:'.length);
      }
      const activeConversationId = conversationId ?? (
        targetKind === 'companion'
          ? (await ipcBridge.companion.getCompanionSession.invoke({
              companion_id: parseCompanionId(targetId),
            })).conversation_id
          : null
      );
      await agentPlatform.selectProductBinding.invoke({
        target_kind: targetKind,
        target_id: targetId,
        request: {
          preset_id: parseAgentPresetId(presetId),
          ...(activeConversationId ? { conversation_id: activeConversationId } : {}),
        },
      });
      setValue(next);
      Message.success(t('agentSettings.productBinding.saved', { defaultValue: 'Agent 已更新' }));
      onChanged?.();
    } catch (reason) {
      console.error('[ProductAgentBindingSelect] Failed to update binding:', reason);
      Message.error(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setSaving(false);
    }
  }, [conversationId, model, onChanged, saving, t, targetId, targetKind, value]);

  if (isLoading || loadingBinding) return <Spin size={14} />;
  return (
    <Select
      value={value}
      options={options}
      loading={saving}
      disabled={disabled || Boolean(error)}
      onChange={(next) => void change(next)}
      style={{ width: 190 }}
      aria-label={t('agentSettings.productBinding.label', { defaultValue: 'Agent 设定' })}
    />
  );
};

export default ProductAgentBindingSelect;
