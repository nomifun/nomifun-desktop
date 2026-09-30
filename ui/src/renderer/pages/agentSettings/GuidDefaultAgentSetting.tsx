/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { configService } from '@/common/config/configService';
import type { GuidAgentSelectionPreference } from '@/common/config/configKeys';
import type { AgentPresetLibraryResponse } from '@/common/types/agentPlatform';
import {
  filterConversationAgentPresets,
  isConversationAgentTemplate,
} from '@/renderer/components/agent/conversationAgentCatalog';
import { useConfig } from '@/renderer/hooks/config/useConfig';
import { SettingTwo } from '@icon-park/react';
import { useId, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  DEFAULT_GUID_AGENT_SELECTION,
  normalizeGuidAgentSelection,
} from '../guid/hooks/agentSelectionUtils';
import type { GuidAgentSelection } from '../guid/types';
import { TEMPLATE_I18N_PATH } from './model';
import styles from './AgentSettingsPage.module.css';

type Props = {
  library: AgentPresetLibraryResponse;
};

type DefaultAgentOption = {
  label: string;
  selection: GuidAgentSelection;
  value: string;
};

const selectionValue = (selection: GuidAgentSelection): string =>
  selection.kind === 'template'
    ? `template:${selection.templateKey}`
    : `preset:${selection.presetId}`;

/** Configure the Agent applied when an explicit new Guid conversation starts. */
export default function GuidDefaultAgentSetting({ library }: Props) {
  const { t } = useTranslation();
  const [storedDefault, setStoredDefault] = useConfig('guid.defaultAgentSelection');
  const [legacySelection] = useConfig('guid.agentSelection');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const feedbackId = useId();

  const templateOptions = useMemo<DefaultAgentOption[]>(() =>
    library.official_templates
      .filter(isConversationAgentTemplate)
      .map((template) => {
        const selection: GuidAgentSelection = {
          kind: 'template',
          templateKey: template.template_key,
        };
        return {
          selection,
          value: selectionValue(selection),
          label: t(
            `agentSettings.template.${TEMPLATE_I18N_PATH[template.template_key]}.name`
          ),
        };
      }), [library.official_templates, t]);

  const presetOptions = useMemo<DefaultAgentOption[]>(() =>
    filterConversationAgentPresets(
      library.user_presets,
      library.active_bindings
    )
      .filter((preset) => Boolean(preset.current_stable_revision))
      .map((preset) => {
        const selection: GuidAgentSelection = {
          kind: 'preset',
          presetId: preset.preset_id,
        };
        return {
          selection,
          value: selectionValue(selection),
          label: preset.display_name,
        };
      }), [library.active_bindings, library.user_presets]);

  const options = useMemo(
    () => [...templateOptions, ...presetOptions],
    [presetOptions, templateOptions]
  );
  const optionByValue = useMemo(
    () => new Map(options.map((option) => [option.value, option])),
    [options]
  );
  const configuredSelection = normalizeGuidAgentSelection(
    storedDefault ?? legacySelection
  );
  const configuredValue = selectionValue(configuredSelection);
  const fallbackValue = selectionValue(DEFAULT_GUID_AGENT_SELECTION);
  const effectiveValue = optionByValue.has(configuredValue)
    ? configuredValue
    : optionByValue.has(fallbackValue)
      ? fallbackValue
      : '';
  const effectiveOption = optionByValue.get(effectiveValue);
  const configuredDefaultUnavailable =
    (storedDefault !== undefined || legacySelection !== undefined)
    && !optionByValue.has(configuredValue);

  const save = async (value: string) => {
    const option = optionByValue.get(value);
    if (!option || saving || value === effectiveValue) return;
    const previous = storedDefault;
    setSaving(true);
    setError(null);
    try {
      await setStoredDefault(option.selection as GuidAgentSelectionPreference);
    } catch {
      configService.setLocal('guid.defaultAgentSelection', previous);
      setError(t('agentSettings.defaultAgent.saveFailed'));
    } finally {
      setSaving(false);
    }
  };

  const currentName = effectiveOption?.label
    ?? t('agentSettings.defaultAgent.noAvailableShort');
  const feedback = error ?? (configuredDefaultUnavailable && effectiveOption
    ? t('agentSettings.defaultAgent.unavailable', { name: effectiveOption.label })
    : null);

  return <div className={styles.defaultAgentSetting}>
    <label className={styles.defaultAgentField} title={t('agentSettings.defaultAgent.hint')}>
      <SettingTwo theme='outline' size={15} />
      <span>{t('agentSettings.defaultAgent.title')}</span>
      <select
        className={styles.defaultAgentSelect}
        aria-label={t('agentSettings.defaultAgent.title')}
        aria-describedby={feedback ? feedbackId : undefined}
        aria-busy={saving}
        title={options.length === 0 ? t('agentSettings.defaultAgent.noAvailable') : currentName}
        value={effectiveValue}
        disabled={saving || options.length === 0}
        onChange={(event) => void save(event.currentTarget.value)}
      >
        {!effectiveOption && <option value='' disabled>{t('agentSettings.defaultAgent.noAvailableShort')}</option>}
        {templateOptions.length > 0 && (
          <optgroup label={t('agentSettings.defaultAgent.officialGroup')}>
            {templateOptions.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </optgroup>
        )}
        {presetOptions.length > 0 && (
          <optgroup label={t('agentSettings.defaultAgent.personalGroup')}>
            {presetOptions.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </optgroup>
        )}
      </select>
    </label>
    {feedback && <p id={feedbackId} role={error ? 'alert' : 'status'} className={`${styles.defaultAgentFeedback} ${error ? styles.defaultAgentError : ''}`}>
      {feedback}
    </p>}
  </div>;
}
