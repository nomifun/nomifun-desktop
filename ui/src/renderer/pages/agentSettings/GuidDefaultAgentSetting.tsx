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
import { Alert, Button, Modal } from '@arco-design/web-react';
import { SettingTwo } from '@icon-park/react';
import { useEffect, useMemo, useState } from 'react';
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
  const [open, setOpen] = useState(false);
  const [draftValue, setDraftValue] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  useEffect(() => {
    if (open) setDraftValue(effectiveValue);
  }, [effectiveValue, open]);

  const show = () => {
    setDraftValue(effectiveValue);
    setError(null);
    setOpen(true);
  };

  const save = async () => {
    const option = optionByValue.get(draftValue);
    if (!option) return;
    const previous = storedDefault;
    setSaving(true);
    setError(null);
    try {
      await setStoredDefault(option.selection as GuidAgentSelectionPreference);
      setOpen(false);
    } catch {
      configService.setLocal('guid.defaultAgentSelection', previous);
      setError(t('agentSettings.defaultAgent.saveFailed'));
    } finally {
      setSaving(false);
    }
  };

  const currentName = effectiveOption?.label
    ?? t('agentSettings.defaultAgent.noAvailableShort');
  const buttonLabel = t('agentSettings.defaultAgent.button', {
    name: currentName,
  });

  return <>
    <Button
      className={styles.defaultAgentButton}
      icon={<SettingTwo theme='outline' size={15} />}
      disabled={options.length === 0}
      title={buttonLabel}
      onClick={show}
    >
      <span className={styles.defaultAgentButtonLabel}>{buttonLabel}</span>
    </Button>
    <Modal
      visible={open}
      title={t('agentSettings.defaultAgent.title')}
      footer={null}
      autoFocus
      focusLock
      unmountOnExit
      onCancel={() => { if (!saving) setOpen(false); }}
    >
      <div className={styles.defaultAgentDialog}>
        <p>{t('agentSettings.defaultAgent.hint')}</p>
        {configuredDefaultUnavailable && effectiveOption && (
          <Alert
            type='warning'
            showIcon
            content={t('agentSettings.defaultAgent.unavailable', {
              name: effectiveOption.label,
            })}
          />
        )}
        {error && <Alert type='error' showIcon content={error} />}
        {options.length === 0 ? (
          <Alert
            type='warning'
            showIcon
            content={t('agentSettings.defaultAgent.noAvailable')}
          />
        ) : (
          <label className={styles.defaultAgentField}>
            <span>{t('agentSettings.defaultAgent.field')}</span>
            <select
              className={styles.nativeSelect}
              aria-label={t('agentSettings.defaultAgent.field')}
              value={draftValue}
              disabled={saving}
              onChange={(event) => setDraftValue(event.currentTarget.value)}
            >
              {templateOptions.length > 0 && (
                <optgroup label={t('agentSettings.defaultAgent.officialGroup')}>
                  {templateOptions.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </optgroup>
              )}
              {presetOptions.length > 0 && (
                <optgroup label={t('agentSettings.defaultAgent.personalGroup')}>
                  {presetOptions.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </optgroup>
              )}
            </select>
          </label>
        )}
        <div className={styles.defaultAgentActions}>
          <Button disabled={saving} onClick={() => setOpen(false)}>
            {t('common.cancel')}
          </Button>
          <Button
            type='primary'
            loading={saving}
            disabled={!draftValue || draftValue === configuredValue}
            onClick={() => void save()}
          >
            {t('agentSettings.defaultAgent.save')}
          </Button>
        </div>
      </div>
    </Modal>
  </>;
}
