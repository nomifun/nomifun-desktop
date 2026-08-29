import type {
  AgentPresetLibraryResponse,
  AgentPresetSummary,
  OfficialPresetKey,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import { Button, Empty, Tooltip } from '@arco-design/web-react';
import {
  AddOne,
  Code,
  Customer,
  Edit,
  Magic,
  MessageOne,
  Robot,
  User,
} from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { TEMPLATE_I18N_PATH, templateCapabilityCount } from './model';
import styles from './AgentSettingsPage.module.css';

type Selection =
  | { kind: 'template'; template: OfficialPresetTemplate }
  | { kind: 'preset'; preset: AgentPresetSummary }
  | null;

type AgentPresetLibraryProps = {
  library: AgentPresetLibraryResponse;
  selection: Selection;
  busy: boolean;
  onSelectTemplate: (template: OfficialPresetTemplate) => void;
  onSelectPreset: (preset: AgentPresetSummary) => void;
  onCreatePreset: (displayName: string) => void;
};

const templateIcon = (key: OfficialPresetKey): React.ReactNode => {
  switch (key) {
    case 'chat.minimal':
      return <MessageOne theme='outline' size='17' />;
    case 'assistant.general':
      return <User theme='outline' size='17' />;
    case 'coding.codex':
      return <Code theme='outline' size='17' />;
    case 'companion.default':
      return <User theme='outline' size='17' />;
    case 'robot.default':
      return <Robot theme='outline' size='17' />;
    case 'customer-service.default':
      return <Customer theme='outline' size='17' />;
    case 'creative-studio.default':
      return <Magic theme='outline' size='17' />;
  }
};

const AgentPresetLibrary: React.FC<AgentPresetLibraryProps> = ({
  library,
  selection,
  busy,
  onSelectTemplate,
  onSelectPreset,
  onCreatePreset,
}) => {
  const { t } = useTranslation();
  const createLabel = t('agentSettings.actions.create');

  return (
    <aside className={styles.library} aria-label={t('agentSettings.library.ariaLabel')}>
      <div className={styles.libraryHeader}>
        <div>
          <div className={styles.libraryTitle}>{t('agentSettings.library.title')}</div>
          <div className={styles.librarySubtitle}>{t('agentSettings.library.subtitle')}</div>
        </div>
        <Tooltip content={createLabel}>
          <Button
            type='primary'
            size='small'
            icon={<AddOne theme='outline' size='15' />}
            loading={busy}
            onClick={() => onCreatePreset(t('agentSettings.defaults.untitledName'))}
          >
            {createLabel}
          </Button>
        </Tooltip>
      </div>

      <section className={styles.librarySection}>
        <div className={styles.librarySectionTitle}>
          {t('agentSettings.library.official')}
          <span>{library.official_templates.length}</span>
        </div>
        <div className={styles.libraryList}>
          {library.official_templates.map((template) => {
            const path = TEMPLATE_I18N_PATH[template.template_key];
            const name = t(`agentSettings.template.${path}.name`);
            const selected =
              selection?.kind === 'template' &&
              selection.template.template_key === template.template_key;
            return (
              <div
                key={template.template_key}
                className={`${styles.libraryRow} ${selected ? styles.libraryRowActive : ''}`}
              >
                <button
                  type='button'
                  className={`${styles.librarySelect} ${styles.librarySelectFull}`}
                  onClick={() => onSelectTemplate(template)}
                >
                  <span className={styles.libraryIcon}>{templateIcon(template.template_key)}</span>
                  <span className={styles.libraryCopy}>
                    <span className={styles.libraryName}>{name}</span>
                    <span className={styles.libraryMeta}>
                      {t('agentSettings.library.capabilityCount', {
                        count: templateCapabilityCount(template),
                      })}
                    </span>
                  </span>
                </button>
              </div>
            );
          })}
        </div>
      </section>

      <section className={styles.librarySection}>
        <div className={styles.librarySectionTitle}>
          {t('agentSettings.library.mine')}
          <span>{library.user_presets.length}</span>
        </div>
        {library.user_presets.length === 0 ? (
          <Empty
            className={styles.libraryEmpty}
            description={t('agentSettings.library.empty')}
          />
        ) : (
          <div className={styles.libraryList}>
            {library.user_presets.map((preset) => {
              const selected =
                selection?.kind === 'preset' &&
                selection.preset.preset_id === preset.preset_id;
              return (
                <div
                  key={preset.preset_id}
                  className={`${styles.libraryRow} ${selected ? styles.libraryRowActive : ''}`}
                >
                  <button
                    type='button'
                    className={`${styles.librarySelect} ${styles.librarySelectFull}`}
                    onClick={() => onSelectPreset(preset)}
                  >
                    <span className={styles.libraryIcon}>
                      <Edit theme='outline' size='17' />
                    </span>
                    <span className={styles.libraryCopy}>
                      <span className={styles.libraryName}>{preset.display_name}</span>
                      <span className={styles.libraryMeta}>
                        {preset.current_stable_revision
                          ? t('agentSettings.library.revision', {
                              revision: preset.current_stable_revision.revision,
                            })
                          : t('agentSettings.library.unsaved')}
                        {preset.bound_target_count > 0
                          ? ` · ${t('agentSettings.library.bindingCount', {
                              count: preset.bound_target_count,
                            })}`
                          : ''}
                      </span>
                    </span>
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </section>
    </aside>
  );
};

export default AgentPresetLibrary;
