import type {
  AgentPresetLibraryResponse,
  AgentPresetSummary,
  OfficialPresetKey,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import { Button, Empty, Popconfirm, Tooltip } from '@arco-design/web-react';
import {
  AddOne,
  Code,
  Customer,
  Delete,
  Edit,
  Loading,
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
  creating: boolean;
  openingPresetId: string | null;
  deletingPresetId: string | null;
  onSelectTemplate: (template: OfficialPresetTemplate) => void;
  onSelectPreset: (preset: AgentPresetSummary) => void;
  onCreatePreset: (displayName: string) => void;
  onDeletePreset: (preset: AgentPresetSummary) => void | Promise<void>;
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
  creating,
  openingPresetId,
  deletingPresetId,
  onSelectTemplate,
  onSelectPreset,
  onCreatePreset,
  onDeletePreset,
}) => {
  const { t } = useTranslation();
  const createLabel = t('agentSettings.actions.create');

  return (
    <aside className={styles.library} aria-label={t('agentSettings.library.ariaLabel')}>
      <div className={styles.libraryHeader}>
        <div>
          <div className={styles.libraryTitle}>{t('agentSettings.title')}</div>
        </div>
        <Tooltip content={createLabel}>
          <Button
            type='primary'
            size='small'
            icon={<AddOne theme='outline' size='15' />}
            loading={creating}
            disabled={busy}
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
                  disabled={busy}
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
          <Empty className={styles.libraryEmpty} description={t('agentSettings.library.empty')} />
        ) : (
          <div className={styles.libraryList}>
            {library.user_presets.map((preset) => {
              const selected =
                (selection?.kind === 'preset' &&
                  selection.preset.preset_id === preset.preset_id) ||
                openingPresetId === preset.preset_id;
              return (
                <div
                  key={preset.preset_id}
                  className={`${styles.libraryRow} ${selected ? styles.libraryRowActive : ''}`}
                >
                  <button
                    type='button'
                    className={styles.librarySelect}
                    disabled={busy}
                    aria-busy={openingPresetId === preset.preset_id}
                    onClick={() => onSelectPreset(preset)}
                  >
                    <span className={styles.libraryIcon}>
                      {openingPresetId === preset.preset_id ? (
                        <Loading
                          theme='outline'
                          size='17'
                          className='animate-spin'
                        />
                      ) : (
                        <Edit theme='outline' size='17' />
                      )}
                    </span>
                    <span className={styles.libraryCopy}>
                      <span className={styles.libraryName}>{preset.display_name}</span>
                      {preset.description && (
                        <span className={styles.libraryMeta}>{preset.description}</span>
                      )}
                    </span>
                  </button>
                  <Popconfirm
                      title={t('agentSettings.library.deleteConfirmTitle', {
                        name: preset.display_name,
                      })}
                      content={t('agentSettings.library.deleteConfirmBody')}
                      okText={t('agentSettings.actions.delete')}
                      cancelText={t('common.cancel')}
                      disabled={busy}
                      okButtonProps={{ status: 'danger' }}
                      onOk={() => onDeletePreset(preset)}
                    >
                      <Button
                        type='text'
                        status='danger'
                        size='mini'
                        className={styles.rowAction}
                        aria-label={t('agentSettings.library.deleteAria', {
                          name: preset.display_name,
                        })}
                        title={t('agentSettings.actions.delete')}
                        icon={<Delete theme='outline' size='14' />}
                        loading={deletingPresetId === preset.preset_id}
                        disabled={busy}
                      />
                    </Popconfirm>
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
