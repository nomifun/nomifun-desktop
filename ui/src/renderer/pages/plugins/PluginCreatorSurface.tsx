import type {
  GeneratedPluginDraft,
  PluginProjectDetail,
} from '@/common/types/pluginPlatform';
import { Alert, Button, Input, Progress, Spin, Tag } from '@arco-design/web-react';
import {
  ArrowLeft,
  CheckOne,
  Code,
  Link,
  PlayOne,
  Save,
  Send,
  SettingTwo,
} from '@icon-park/react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { PluginLoadFailure } from './pluginWorkbenchModel';
import styles from './PluginProductSurface.module.css';

export type PluginAiBusyStep = 'understanding' | 'writing' | 'dependencies' | 'building' | null;

export interface PluginCreatorMessage {
  role: 'user' | 'assistant';
  content: string;
}

interface PluginCreatorSurfaceProps {
  title: string;
  messages: PluginCreatorMessage[];
  draft: GeneratedPluginDraft | null;
  project: PluginProjectDetail | null;
  busyStep: PluginAiBusyStep;
  failure: PluginLoadFailure | null;
  modelAvailable: boolean;
  projectBusy: boolean;
  onBack: () => void;
  onSend: (message: string) => void;
  onOpenModels: () => void;
  onBuild: () => void;
  onEditSource: () => void;
  onEditDependencies: () => void;
  onTest: () => void;
  onApply: () => void;
  onExport: () => void;
}

const stepProgress: Record<Exclude<PluginAiBusyStep, null>, number> = {
  understanding: 18,
  writing: 45,
  dependencies: 68,
  building: 88,
};

const PluginCreatorSurface: React.FC<PluginCreatorSurfaceProps> = ({
  title,
  messages,
  draft,
  project,
  busyStep,
  failure,
  modelAvailable,
  projectBusy,
  onBack,
  onSend,
  onOpenModels,
  onBuild,
  onEditSource,
  onEditDependencies,
  onTest,
  onApply,
  onExport,
}) => {
  const { t } = useTranslation();
  const [input, setInput] = useState('');
  const busy = busyStep !== null || projectBusy;
  const ready = project?.ready;

  const submit = () => {
    const value = input.trim();
    if (!value || busy) return;
    setInput('');
    onSend(value);
  };

  return (
    <div className={styles.creator}>
      <header className={styles.creatorHeader}>
        <Button type='text' icon={<ArrowLeft size={16} />} onClick={onBack}>
          {t('pluginWorkbench.product.back')}
        </Button>
        <div className={styles.creatorHeaderCopy}>
          <span className={styles.creatorAvatar}><Code theme='outline' size={17} /></span>
          <div>
            <span className={styles.eyebrow}>{t('pluginWorkbench.product.aiCreator')}</span>
            <h2>{title || t('pluginWorkbench.product.untitled')}</h2>
            <p>{t('pluginWorkbench.product.autoSaved')}</p>
          </div>
        </div>
        <div className={styles.creatorStatus}>
          {busy ? <Spin size={14} /> : <CheckOne theme='filled' size={14} />}
          <span>{busy ? t('pluginWorkbench.product.creating') : t('pluginWorkbench.product.draftSaved')}</span>
        </div>
      </header>

      {!modelAvailable && (
        <Alert
          type='warning'
          showIcon
          content={t('pluginWorkbench.product.modelRequired')}
          action={<Button size='small' onClick={onOpenModels}>{t('pluginWorkbench.product.configureModel')}</Button>}
        />
      )}
      {failure && <Alert type='error' showIcon content={failure.message} />}

      <div className={styles.creatorBody}>
        <main className={styles.chatColumn} aria-label={t('pluginWorkbench.product.aiConversation')}>
          {messages.map((message, index) => (
            message.role === 'user' ? (
              <div key={`${message.role}-${index}`} className={`${styles.chatMessage} ${styles.chatMessageUser}`}>
                {message.content}
              </div>
            ) : (
              <div key={`${message.role}-${index}`} className={`${styles.chatMessage} ${styles.chatMessageAssistant}`}>
                <span className={styles.creatorAvatar}><Code theme='outline' size={15} /></span>
                <div><strong>Nomi</strong><p>{message.content}</p></div>
              </div>
            )
          ))}

          {busyStep && (
            <section className={styles.creationProgress} aria-live='polite'>
              <header>
                <strong>{t(`pluginWorkbench.product.steps.${busyStep}`)}</strong>
                <span>{stepProgress[busyStep]}%</span>
              </header>
              <Progress percent={stepProgress[busyStep]} showText={false} size='small' />
              <div className={styles.progressSteps}>
                <span>{t('pluginWorkbench.product.steps.understanding')}</span>
                <span>{t('pluginWorkbench.product.steps.writing')}</span>
                <span>{t('pluginWorkbench.product.steps.building')}</span>
              </div>
            </section>
          )}

          {!messages.length && !busyStep && (
            <div className={styles.productEmpty}>
              <span><Code theme='outline' size={26} /></span>
              <h2>{t('pluginWorkbench.product.creatorEmptyTitle')}</h2>
              <p>{t('pluginWorkbench.product.creatorEmptyBody')}</p>
            </div>
          )}

          <div className={styles.chatComposer}>
            <Input.TextArea
              value={input}
              disabled={busy || !modelAvailable}
              autoSize={{ minRows: 2, maxRows: 5 }}
              placeholder={t('pluginWorkbench.product.adjustPlaceholder')}
              onChange={setInput}
              onPressEnter={(event: React.KeyboardEvent<HTMLTextAreaElement>) => {
                if (!event.shiftKey && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  submit();
                }
              }}
            />
            <Button
              type='primary'
              icon={<Send size={15} />}
              loading={busyStep !== null}
              disabled={!input.trim() || busy || !modelAvailable}
              onClick={submit}
            >
              {t('pluginWorkbench.product.send')}
            </Button>
          </div>
        </main>

        <aside className={styles.creatorSidebar} aria-label={t('pluginWorkbench.product.currentEnhancements')}>
          <h2>{t('pluginWorkbench.product.currentEnhancements')}</h2>
          <p>{t('pluginWorkbench.product.currentEnhancementsHint')}</p>
          <div className={styles.capabilityList}>
            {(draft?.capabilities ?? []).map((capability) => (
              <div key={capability.capability_id} className={styles.capabilityRow}>
                <span className={styles.capabilityIcon}><Link theme='outline' size={15} /></span>
                <div>
                  <strong>{capability.display_name}</strong>
                  <small>{capability.description}</small>
                </div>
              </div>
            ))}
            {!draft?.capabilities.length && (
              <div className={styles.capabilityRow}>
                <span className={styles.capabilityIcon}><Code theme='outline' size={15} /></span>
                <div>
                  <strong>{t('pluginWorkbench.product.waitingForCapabilities')}</strong>
                  <small>{t('pluginWorkbench.product.waitingForCapabilitiesHint')}</small>
                </div>
              </div>
            )}
          </div>

          <div className={styles.systemCooperation}>
            <strong>{t('pluginWorkbench.product.systemCooperation')}</strong>
            <span>{t('pluginWorkbench.product.systemInput')}</span>
            <span>{t('pluginWorkbench.product.pluginEnhancement')}</span>
            <span>{t('pluginWorkbench.product.systemOutput')}</span>
          </div>

          {ready && (
            <section className={styles.previewPanel}>
              <div className={styles.previewHeader}>
                <div>
                  <h2>{t('pluginWorkbench.product.readyTitle')}</h2>
                  <p>{t('pluginWorkbench.product.readyBody')}</p>
                </div>
                <Tag color={ready.test.status === 'passed' ? 'green' : 'orange'}>
                  {t(`pluginWorkbench.testStatus.${ready.test.status === 'needs_test_input' ? 'needsInput' : ready.test.status === 'not_run' ? 'notRun' : ready.test.status}`)}
                </Tag>
              </div>
              <div className={styles.previewFlow}>
                <div><strong>{t('pluginWorkbench.product.flow.request')}</strong><small>Nomi</small></div>
                <div><strong>{t('pluginWorkbench.product.flow.system')}</strong><small>{t('pluginWorkbench.product.flow.builtin')}</small></div>
                <div data-plugin='true'><strong>{t('pluginWorkbench.product.flow.plugin')}</strong><small>{draft?.capabilities[0]?.display_name ?? title}</small></div>
                <div><strong>{t('pluginWorkbench.product.flow.result')}</strong><small>{t('pluginWorkbench.product.flow.consumer')}</small></div>
              </div>
              <Button icon={<PlayOne size={15} />} disabled={busy} onClick={onTest}>
                {t('pluginWorkbench.product.runCheck')}
              </Button>
              <Button type='primary' icon={<Save size={15} />} disabled={busy || !ready.impact.can_apply} onClick={onApply}>
                {t('pluginWorkbench.product.saveAndEnable')}
              </Button>
            </section>
          )}

          <details className={styles.creatorAdvanced}>
            <summary>{t('common.technical_details')}</summary>
            <div className={styles.advancedActions}>
              <Button size='small' icon={<Code size={14} />} disabled={!project || busy} onClick={onEditSource}>
                {t('pluginWorkbench.actions.editSource')}
              </Button>
              <Button size='small' icon={<SettingTwo size={14} />} disabled={!project || busy} onClick={onEditDependencies}>
                {t('pluginWorkbench.actions.editDependencies')}
              </Button>
              <Button size='small' disabled={!project || busy} onClick={onBuild}>
                {t('pluginWorkbench.actions.build')}
              </Button>
              <Button size='small' disabled={!project || busy || (!ready && !project?.summary.linked_mount_id)} onClick={onExport}>
                {t('pluginWorkbench.actions.exportShare')}
              </Button>
            </div>
          </details>
        </aside>
      </div>
    </div>
  );
};

export default PluginCreatorSurface;
