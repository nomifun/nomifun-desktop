/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  ConfigurePluginRequest,
  PluginCredentialBindingStatus,
  PluginCredentialSlotBinding,
  PluginDetail,
} from '@/common/types/pluginPlatform';
import {
  Alert,
  Checkbox,
  Input,
  InputNumber,
  Modal,
  type ModalProps,
  Radio,
  Select,
  Switch,
} from '@arco-design/web-react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  configurePluginRequest,
  createPluginConfigurationDraft,
  pluginConfigurationEditorModel,
  validatePluginConfigurationDraft,
  type PluginConfigFieldModel,
  type PluginConfigScalar,
  type PluginConfigSchemaIssue,
  type PluginConfigurationDraft,
  type PluginConfigurationDraftIssue,
  type PluginCredentialDraftAction,
} from './pluginConfigurationModel';
import styles from './PluginWorkbenchPage.module.css';

export interface PluginConfigurationDialogProps {
  visible: boolean;
  detail: PluginDetail | null;
  loading: boolean;
  failure?: { message: string } | null;
  onCancel: () => void;
  onSubmit: (request: ConfigurePluginRequest) => void | Promise<void>;
}

type Translator = ReturnType<typeof useTranslation>['t'];
const ConfigurationModal =
  Modal as unknown as React.ComponentType<React.PropsWithChildren<ModalProps>>;

const schemaIssueMessage = (
  issue: PluginConfigSchemaIssue,
  t: Translator
): string => {
  const values = { path: issue.path, keyword: issue.keyword ?? '' };
  switch (issue.code) {
    case 'root_not_object':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.rootNotObject', values);
    case 'root_type_unsupported':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.rootType', values);
    case 'properties_not_object':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.properties', values);
    case 'required_invalid':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.required', values);
    case 'required_property_missing':
      return t(
        'pluginWorkbench.dialogs.configure.schemaIssues.requiredPropertyMissing',
        values
      );
    case 'dynamic_properties_unsupported':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.dynamicProperties', values);
    case 'unsupported_keyword':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.unsupportedKeyword', values);
    case 'field_not_object':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.fieldNotObject', values);
    case 'field_type_unsupported':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.fieldType', values);
    case 'field_enum_invalid':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.enumInvalid', values);
    case 'field_constraint_invalid':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.constraintInvalid', values);
    case 'field_read_only':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.readOnly', values);
    case 'secret_config_unsupported':
      return t(
        'pluginWorkbench.dialogs.configure.schemaIssues.secretConfig',
        values
      );
    case 'schema_digest_mismatch':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.digestMismatch', values);
    case 'duplicate_credential_slot':
      return t(
        'pluginWorkbench.dialogs.configure.schemaIssues.duplicateCredentialSlot',
        values
      );
    case 'mount_target_missing':
      return t('pluginWorkbench.dialogs.configure.schemaIssues.noCurrentTarget', values);
  }
};

const draftIssueMessage = (
  issue: PluginConfigurationDraftIssue,
  t: Translator
): string => {
  const values = { limit: issue.limit ?? 0 };
  switch (issue.code) {
    case 'required':
      return t('pluginWorkbench.dialogs.configure.fieldErrors.required', values);
    case 'invalid_type':
      return t('pluginWorkbench.dialogs.configure.fieldErrors.invalidType', values);
    case 'integer_required':
      return t('pluginWorkbench.dialogs.configure.fieldErrors.integer', values);
    case 'minimum':
      return t('pluginWorkbench.dialogs.configure.fieldErrors.minimum', values);
    case 'maximum':
      return t('pluginWorkbench.dialogs.configure.fieldErrors.maximum', values);
    case 'min_length':
      return t('pluginWorkbench.dialogs.configure.fieldErrors.minLength', values);
    case 'max_length':
      return t('pluginWorkbench.dialogs.configure.fieldErrors.maxLength', values);
    case 'enum_value':
      return t('pluginWorkbench.dialogs.configure.fieldErrors.enumValue', values);
    case 'credential_required':
      return t('pluginWorkbench.dialogs.configure.fieldErrors.credentialRequired', values);
    case 'credential_id_required':
      return t('pluginWorkbench.dialogs.configure.fieldErrors.credentialIdRequired', values);
  }
};

const statusLabel = (
  status: PluginCredentialBindingStatus,
  t: Translator
): string => {
  const labels: Record<PluginCredentialBindingStatus, string> = {
    unbound: t('pluginWorkbench.dialogs.configure.credentialStatus.unbound'),
    bound: t('pluginWorkbench.dialogs.configure.credentialStatus.bound'),
    missing: t('pluginWorkbench.dialogs.configure.credentialStatus.missing'),
    invalid: t('pluginWorkbench.dialogs.configure.credentialStatus.invalid'),
  };
  return labels[status];
};

const defaultFieldValue = (field: PluginConfigFieldModel): PluginConfigScalar => {
  if (field.defaultValue !== undefined) return field.defaultValue;
  if (field.kind === 'enum') return field.enumOptions[0]?.value ?? '';
  if (field.kind === 'boolean') return false;
  if (field.kind === 'string') return '';
  const boundedZero =
    field.minimum !== undefined && field.minimum > 0
      ? field.minimum
      : field.maximum !== undefined && field.maximum < 0
        ? field.maximum
        : 0;
  if (field.kind !== 'integer') return boundedZero;
  return boundedZero > 0 ? Math.ceil(boundedZero) : Math.floor(boundedZero);
};

const fieldIssueMessages = (
  issues: PluginConfigurationDraftIssue[],
  scope: PluginConfigurationDraftIssue['scope'],
  key: string,
  t: Translator
): string[] =>
  issues
    .filter((issue) => issue.scope === scope && issue.key === key)
    .map((issue) => draftIssueMessage(issue, t));

const PluginConfigFieldControl: React.FC<{
  field: PluginConfigFieldModel;
  draft: PluginConfigurationDraft;
  disabled: boolean;
  issues: string[];
  onValue: (value: PluginConfigScalar | undefined) => void;
}> = ({ field, draft, disabled, issues, onValue }) => {
  const { t } = useTranslation();
  const value = draft.values[field.key];
  const configured = value !== undefined;
  const controlDisabled = disabled || (!field.required && !configured);

  let control: React.ReactNode;
  if (field.kind === 'enum') {
    const selected = field.enumOptions.find((option) =>
      Object.is(option.value, value)
    );
    control = (
      <Select
        value={selected?.id}
        disabled={controlDisabled}
        onChange={(id: string) =>
          onValue(field.enumOptions.find((option) => option.id === id)?.value)
        }
        placeholder={t('pluginWorkbench.dialogs.configure.selectValue')}
        aria-label={field.label}
      >
        {field.enumOptions.map((option) => (
          <Select.Option key={option.id} value={option.id}>
            {option.label}
          </Select.Option>
        ))}
      </Select>
    );
  } else if (field.kind === 'boolean') {
    control = (
      <Switch
        checked={value === true}
        disabled={controlDisabled}
        onChange={(next: boolean) => onValue(next)}
        checkedText={t('pluginWorkbench.dialogs.configure.booleanOn')}
        uncheckedText={t('pluginWorkbench.dialogs.configure.booleanOff')}
        aria-label={field.label}
      />
    );
  } else if (field.kind === 'number' || field.kind === 'integer') {
    control = (
      <InputNumber
        value={typeof value === 'number' ? value : undefined}
        disabled={controlDisabled}
        min={field.minimum}
        max={field.maximum}
        precision={field.kind === 'integer' ? 0 : undefined}
        onChange={(next: number) =>
          onValue(Number.isFinite(next) ? next : undefined)
        }
        aria-label={field.label}
      />
    );
  } else {
    control = (
      <Input
        value={typeof value === 'string' ? value : ''}
        disabled={controlDisabled}
        maxLength={field.maxLength}
        onChange={onValue}
        aria-label={field.label}
      />
    );
  }

  return (
    <div className={styles.configurationField}>
      <div className={styles.configurationFieldHeader}>
        <div className={styles.configurationFieldCopy}>
          <span className={styles.configurationFieldLabel}>
            {field.label}
            <span className={styles.configurationRequirement}>
              {field.required
                ? t('pluginWorkbench.dialogs.configure.required')
                : t('pluginWorkbench.dialogs.configure.optional')}
            </span>
          </span>
          <span className={`${styles.configurationFieldKey} ${styles.mono}`}>
            {field.key}
          </span>
        </div>
        {!field.required && (
          <Checkbox
            checked={configured}
            disabled={disabled}
            onChange={(checked: boolean) =>
              onValue(checked ? defaultFieldValue(field) : undefined)
            }
          >
            {t('pluginWorkbench.dialogs.configure.useValue')}
          </Checkbox>
        )}
      </div>
      {field.description && (
        <div className={styles.configurationFieldDescription}>
          {field.description}
        </div>
      )}
      <div className={styles.configurationControl}>{control}</div>
      {issues.map((issue) => (
        <div key={issue} className={styles.configurationFieldError}>
          {issue}
        </div>
      ))}
    </div>
  );
};

const CredentialSlotControl: React.FC<{
  slot: PluginCredentialSlotBinding;
  draft: PluginConfigurationDraft;
  disabled: boolean;
  issues: string[];
  onChange: (action: PluginCredentialDraftAction, credentialId?: string) => void;
}> = ({ slot, draft, disabled, issues, onChange }) => {
  const { t } = useTranslation();
  const credential = draft.credentials[slot.slot_key];
  return (
    <div className={styles.configurationField}>
      <div className={styles.configurationFieldHeader}>
        <div className={styles.configurationFieldCopy}>
          <span className={styles.configurationFieldLabel}>
            {slot.display_name}
            <span className={styles.configurationRequirement}>
              {slot.required
                ? t('pluginWorkbench.dialogs.configure.required')
                : t('pluginWorkbench.dialogs.configure.optional')}
            </span>
          </span>
          <span className={`${styles.configurationFieldKey} ${styles.mono}`}>
            {slot.slot_key}
          </span>
        </div>
        <span
          className={`${styles.credentialStatus} ${
            styles[`credentialStatus_${slot.status}`]
          }`}
        >
          {statusLabel(slot.status, t)}
        </span>
      </div>
      {slot.credential_id && (
        <div className={styles.configurationCurrentBinding}>
          <span>{t('pluginWorkbench.dialogs.configure.currentCredential')}</span>
          <span className={styles.mono} title={slot.credential_id}>
            {slot.credential_id}
          </span>
        </div>
      )}
      <Radio.Group
        type='button'
        size='small'
        value={credential.action}
        disabled={disabled}
        onChange={(next: PluginCredentialDraftAction) => onChange(next)}
        aria-label={t('pluginWorkbench.dialogs.configure.credentialActionAria', {
          name: slot.display_name,
        })}
      >
        {slot.credential_id && (
          <Radio value='keep'>
            {t('pluginWorkbench.dialogs.configure.credentialKeep')}
          </Radio>
        )}
        <Radio value='bind'>
          {t('pluginWorkbench.dialogs.configure.credentialBind')}
        </Radio>
        <Radio value='unbind' disabled={slot.required}>
          {t('pluginWorkbench.dialogs.configure.credentialUnbind')}
        </Radio>
      </Radio.Group>
      {credential.action === 'bind' && (
        <Input
          value={credential.credentialId}
          disabled={disabled}
          onChange={(next: string) => onChange('bind', next)}
          placeholder={t('pluginWorkbench.dialogs.configure.credentialIdPlaceholder')}
          aria-label={t('pluginWorkbench.dialogs.configure.credentialIdAria', {
            name: slot.display_name,
          })}
          className={styles.monoInput}
          spellCheck={false}
          autoComplete='off'
        />
      )}
      {!slot.credential_id && credential.action === 'unbind' && (
        <span className={styles.configurationHint}>
          {t('pluginWorkbench.dialogs.configure.credentialUnbound')}
        </span>
      )}
      {issues.map((issue) => (
        <div key={issue} className={styles.configurationFieldError}>
          {issue}
        </div>
      ))}
    </div>
  );
};

const PluginConfigurationDialog: React.FC<PluginConfigurationDialogProps> = ({
  visible,
  detail,
  loading,
  failure,
  onCancel,
  onSubmit,
}) => {
  const { t } = useTranslation();
  const editor = useMemo(
    () => (detail ? pluginConfigurationEditorModel(detail) : null),
    [detail]
  );
  const [draft, setDraft] = useState<PluginConfigurationDraft | null>(null);
  const [draftIssues, setDraftIssues] = useState<
    PluginConfigurationDraftIssue[]
  >([]);

  useEffect(() => {
    if (!visible || !detail || !editor) return;
    setDraft(createPluginConfigurationDraft(detail, editor));
    setDraftIssues([]);
  }, [
    detail,
    detail?.config.config_revision,
    detail?.credential_bindings_revision,
    detail?.summary.mount_id,
    detail?.summary.mount_revision,
    editor,
    visible,
  ]);

  const clearIssues = (
    scope: PluginConfigurationDraftIssue['scope'],
    key: string
  ) => {
    setDraftIssues((current) =>
      current.filter((issue) => issue.scope !== scope || issue.key !== key)
    );
  };

  const setValue = (key: string, value: PluginConfigScalar | undefined) => {
    clearIssues('config', key);
    setDraft((current) =>
      current
        ? { ...current, values: { ...current.values, [key]: value } }
        : current
    );
  };

  const setCredential = (
    key: string,
    action: PluginCredentialDraftAction,
    credentialId?: string
  ) => {
    clearIssues('credential', key);
    setDraft((current) => {
      if (!current) return current;
      const previous = current.credentials[key];
      return {
        ...current,
        credentials: {
          ...current.credentials,
          [key]: {
            action,
            credentialId: credentialId ?? previous?.credentialId ?? '',
          },
        },
      };
    });
  };

  const handleSubmit = async () => {
    if (!detail || !editor || !draft || !editor.canSubmit) return;
    const issues = validatePluginConfigurationDraft(detail, editor, draft);
    setDraftIssues(issues);
    if (issues.length > 0) return;
    try {
      await onSubmit(configurePluginRequest(detail, editor, draft));
    } catch {
      // The parent renders request failures while this draft remains mounted.
    }
  };

  const editable = Boolean(editor?.canSubmit);

  return (
    <ConfigurationModal
      visible={visible}
      title={t('pluginWorkbench.dialogs.configure.title')}
      onCancel={loading ? undefined : onCancel}
      onOk={() => void handleSubmit()}
      okText={t('pluginWorkbench.actions.saveConfiguration')}
      cancelText={t('pluginWorkbench.actions.cancel')}
      confirmLoading={loading}
      okButtonProps={{ disabled: !detail || !draft || !editable }}
      maskClosable={false}
      autoFocus={false}
      unmountOnExit
      className={styles.configurationModal}
      style={{ width: 760 }}
    >
      <div className={styles.dialogIntro}>
        {t('pluginWorkbench.dialogs.configure.body', {
          name: detail?.summary.display_name ?? '',
        })}
      </div>
      <div className={styles.configurationDialogBody}>
        {failure && <Alert type='error' showIcon content={failure.message} />}
        {detail && detail.config.validation_errors.length > 0 && (
          <Alert
            type='warning'
            showIcon
            title={t('pluginWorkbench.dialogs.configure.persistedValidationTitle')}
            content={
              <ul className={styles.configurationIssueList}>
                {detail.config.validation_errors.map((error) => (
                  <li key={error}>{error}</li>
                ))}
              </ul>
            }
          />
        )}
        {editor && editor.schemaIssues.length > 0 && (
          <Alert
            type='warning'
            showIcon
            title={t('pluginWorkbench.dialogs.configure.schemaUnsupported')}
            content={
              <ul className={styles.configurationIssueList}>
                {editor.schemaIssues.map((issue, index) => (
                  <li key={`${issue.code}:${issue.path}:${index}`}>
                    {schemaIssueMessage(issue, t)}
                  </li>
                ))}
              </ul>
            }
          />
        )}

        <section className={styles.configurationSection}>
          <div className={styles.configurationSectionHeader}>
            <div>
              <h3>{editor?.title ?? t('pluginWorkbench.dialogs.configure.configTitle')}</h3>
              <p>
                {editor?.description ??
                  t('pluginWorkbench.dialogs.configure.configHint')}
              </p>
            </div>
            {detail && (
              <span className={styles.configurationRevision}>
                r{detail.config.config_revision}
              </span>
            )}
          </div>
          {editor && draft && editor.fields.length > 0 ? (
            <div className={styles.configurationFieldList}>
              {editor.fields.map((field) => (
                <PluginConfigFieldControl
                  key={field.key}
                  field={field}
                  draft={draft}
                  disabled={!editable || loading}
                  issues={fieldIssueMessages(
                    draftIssues,
                    'config',
                    field.key,
                    t
                  )}
                  onValue={(value) => setValue(field.key, value)}
                />
              ))}
            </div>
          ) : (
            <div className={styles.configurationEmpty}>
              {t('pluginWorkbench.dialogs.configure.noConfigFields')}
            </div>
          )}
        </section>

        <section className={styles.configurationSection}>
          <div className={styles.configurationSectionHeader}>
            <div>
              <h3>{t('pluginWorkbench.dialogs.configure.credentialsTitle')}</h3>
              <p>{t('pluginWorkbench.dialogs.configure.credentialsHint')}</p>
            </div>
            {detail && (
              <span className={styles.configurationRevision}>
                r{detail.credential_bindings_revision}
              </span>
            )}
          </div>
          <Alert
            type='info'
            showIcon
            content={t('pluginWorkbench.dialogs.configure.credentialListBoundary')}
          />
          {detail && draft && detail.credential_slots.length > 0 ? (
            <div className={styles.configurationFieldList}>
              {detail.credential_slots.map((slot) => (
                <CredentialSlotControl
                  key={slot.slot_key}
                  slot={slot}
                  draft={draft}
                  disabled={!editable || loading}
                  issues={fieldIssueMessages(
                    draftIssues,
                    'credential',
                    slot.slot_key,
                    t
                  )}
                  onChange={(action, credentialId) =>
                    setCredential(slot.slot_key, action, credentialId)
                  }
                />
              ))}
            </div>
          ) : (
            <div className={styles.configurationEmpty}>
              {t('pluginWorkbench.dialogs.configure.noCredentialSlots')}
            </div>
          )}
        </section>
      </div>
    </ConfigurationModal>
  );
};

export default PluginConfigurationDialog;
