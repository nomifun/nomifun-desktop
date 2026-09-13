import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import { Alert, Button, Input, Spin } from '@arco-design/web-react';
import { ArrowLeft, Send, PreviewOpen } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { parsePluginRuntimeId } from '@/common/types/ids';
import {
  pluginRuntimeProduct,
  type PluginRuntimeDraft,
} from '@/common/adapter/pluginRuntimeProductBridge';
import { useGuidModelSelection } from '@/renderer/pages/guid/hooks/useGuidModelSelection';
import PluginRuntimeDraftPreview from './PluginRuntimeDraftPreview';
import { libraryChanged } from './libraryState';
import styles from './PluginRuntimeProduct.module.css';

export default function PluginRuntimeCreatorPage({
  embedded = false,
}: {
  embedded?: boolean;
}) {
  const { t } = useTranslation(),
    navigate = useNavigate();
  const { draftId } = useParams<{ draftId: string }>();
  const [params] = useSearchParams();
  const appId = params.get('app');
  const { current_model } = useGuidModelSelection('nomi');
  const inputKey = `nomifun-plugin-unsent:${draftId || appId || 'new'}`;
  const [draft, setDraft] = useState<PluginRuntimeDraft | null>(null),
    [input, setInput] = useState(() => {
      try {
        return sessionStorage.getItem(inputKey) ?? params.get('requirement') ?? '';
      } catch {
        return '';
      }
    });
  const [initial, setInitial] = useState({ name: '', html: '' }),
    [busy, setBusy] = useState(false),
    [error, setError] = useState('');
  const [expanded, setExpanded] = useState(false),
    [previewValid, setPreviewValid] = useState(false),
    [previewError, setPreviewError] = useState('');
  const [loading, setLoading] = useState(Boolean(draftId || appId));
  const repairCount = useRef(0),
    sending = useRef(false),
    lastGood = useRef<string>('');
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    try {
      if (input) sessionStorage.setItem(inputKey, input);
      else sessionStorage.removeItem(inputKey);
    } catch {
      /* Editing still works if browser storage is unavailable. */
    }
  }, [input, inputKey]);
  const applyDraft = useCallback(
    (next: PluginRuntimeDraft) =>
      setDraft((current) =>
        current?.id === next.id && current.revision > next.revision
          ? current
          : next,
      ),
    [],
  );
  useEffect(() => {
    let active = true;
    setLoading(Boolean(draftId || appId));
    if (draftId)
      void pluginRuntimeProduct.draft
        .invoke({ id: draftId })
        .then((value) => {
          if (active) applyDraft(value);
        })
        .catch(() => {
          if (active) setError(t('pluginRuntime.product.loadFailed'));
        })
        .finally(() => {
          if (active) setLoading(false);
        });
    else if (appId) {
      const id = parsePluginRuntimeId(appId);
      void Promise.all([
        ipcBridge.pluginRuntimes.getWorkshop.invoke({ plugin_id: id }),
        ipcBridge.pluginRuntimes.getSourceFile.invoke({
          plugin_id: id,
          path: 'ui/index.html',
        }).catch((error: unknown) => {
          if (isBackendHttpError(error) && error.status === 404) return { content: '' };
          throw error;
        }),
      ])
        .then(([app, file]) => {
          if (active)
            setInitial({ name: app.plugin.display_name, html: file.content });
        })
        .catch(() => {
          if (active) setError(t('pluginRuntime.product.cannotEdit'));
        })
        .finally(() => {
          if (active) setLoading(false);
        });
    }
    return () => {
      active = false;
    };
  }, [draftId, appId, t, applyDraft]);
  useEffect(() => {
    if (!draft || draft.status !== 'generating') return;
    let active = true;
    let timer: number;
    const id = draft.id;
    const poll = async () => {
      try {
        const value = await pluginRuntimeProduct.draft.invoke({ id });
        if (!active) return;
        applyDraft(value);
        setError('');
        if (value.status !== 'generating') {
          libraryChanged();
          return;
        }
      } catch {
        if (active) setError(t('pluginRuntime.product.connectionLost'));
      }
      if (active) timer = window.setTimeout(() => void poll(), 2000);
    };
    timer = window.setTimeout(() => void poll(), 1200);
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [draft?.id, draft?.status, applyDraft, t]);
  const html = draft?.html || initial.html;
  useEffect(() => {
    setPreviewValid(false);
    setPreviewError('');
  }, [html]);
  const generating = busy || draft?.status === 'generating';
  const send = useCallback(
    async (text: string, repair = false) => {
      const requirement = text.trim();
      if (!requirement || generating || sending.current) return;
      if (!current_model) {
        setError(t('pluginRuntime.product.modelRequired'));
        return;
      }
      sending.current = true;
      setBusy(true);
      setError('');
      if (!repair) repairCount.current = 0;
      try {
        const next = await pluginRuntimeProduct.generate.invoke({
          provider_id: String(current_model.id),
          model: current_model.use_model,
          requirement,
          ...(draft
            ? { draft_id: draft.id, expected_revision: draft.revision }
            : appId
              ? { plugin_id: appId }
              : {}),
        });
        if (!mounted.current) return;
        try {
          sessionStorage.removeItem(inputKey);
        } catch {
          /* optional unsent-input retention */
        }
        applyDraft(next);
        setInput('');
        libraryChanged();
        if (!draftId)
          navigate(`/plugins/create/${next.id}`, { replace: true });
      } catch {
        if (mounted.current) setError(t('pluginRuntime.product.generateFailed'));
      } finally {
        sending.current = false;
        if (mounted.current) setBusy(false);
      }
    },
    [
      generating,
      current_model,
      draft,
      appId,
      draftId,
      inputKey,
      navigate,
      applyDraft,
      t,
    ],
  );
  const submittedRequirement = useRef<string | null>(null);
  const initialRequirement = params.get('requirement');
  useEffect(() => {
    if (!draftId && !appId && !draft && current_model && initialRequirement && submittedRequirement.current !== initialRequirement) {
      submittedRequirement.current = initialRequirement;
      void send(initialRequirement);
    }
  }, [draftId, appId, draft, current_model, initialRequirement, send]);
  const onPreviewStatus = useCallback(
    (failure: string | null) => {
      if (previewError && lastGood.current && html !== lastGood.current) return;
      if (!failure) {
        lastGood.current = html;
        setPreviewValid(true);
        return;
      }
      setPreviewValid(false);
      setPreviewError(failure);
      if (
        draft?.status === 'ready' &&
        !draft.import &&
        repairCount.current < 1 &&
        current_model
      ) {
        repairCount.current++;
        void send(t('pluginRuntime.product.repairPrompt', { error: failure }), true);
      }
    },
    [html, draft, current_model, send, t, previewError],
  );
  const stop = async () => {
    if (!draft || busy) return;
    try {
      applyDraft(
        await pluginRuntimeProduct.cancel.invoke({
          id: draft.id,
          expected_revision: draft.revision,
        }),
      );
      libraryChanged();
    } catch {
      setError(t('pluginRuntime.product.operationFailed'));
    }
  };
  const save = async () => {
    if (!draft || generating || (html ? !previewValid || previewError : !draft.service_source)) return;
    setBusy(true);
    setError('');
    try {
      const result = await pluginRuntimeProduct.save.invoke({
        id: draft.id,
        expected_revision: draft.revision,
      });
      libraryChanged();
      navigate(`/plugins/run/${result.plugin.plugin_id}?saved=1`);
    } catch {
      setError(t('pluginRuntime.product.saveFailed'));
      try {
        applyDraft(await pluginRuntimeProduct.draft.invoke({ id: draft.id }));
      } catch {
        /* retain the recoverable draft */
      }
    } finally {
      if (mounted.current) setBusy(false);
    }
  };
  const errorNotice = (error || draft?.error) && (
    <Alert
      type='error'
      content={
        error ||
        t(
          draft?.error === 'model_busy'
            ? 'pluginRuntime.product.modelBusy'
            : 'pluginRuntime.product.generateFailed',
        )
      }
      action={
        <Button
          size='small'
          disabled={generating}
          onClick={() => {
            const previous = [...(draft?.messages ?? [])]
              .reverse()
              .find((m) => m.role === 'user');
            void send(
              input || previous?.content || t('pluginRuntime.product.retryPrompt'),
            );
          }}
        >
          {t('pluginRuntime.product.retry')}
        </Button>
      }
    />
  );
  const modelNotice = !current_model && (
    <Alert
      type='warning'
      content={t('pluginRuntime.product.modelRequired')}
      action={
        <Button size='small' onClick={() => navigate('/settings/model')}>
          {t('pluginRuntime.product.connectModel')}
        </Button>
      }
    />
  );
  const composer = (
    <div className={styles.composer}>
      <Input.TextArea
        aria-label={t('pluginRuntime.product.requirement')}
        value={input}
        onChange={setInput}
        disabled={generating}
        autoSize={{ minRows: 3, maxRows: 7 }}
        placeholder={t(
          html
            ? 'pluginRuntime.product.changePlaceholder'
            : 'pluginRuntime.product.requirementPlaceholder',
        )}
        onKeyDown={(event) => {
          if (
            event.key === 'Enter' &&
            !event.shiftKey &&
            !event.nativeEvent.isComposing
          ) {
            event.preventDefault();
            void send(input);
          }
        }}
      />
      <div className={styles.composerActions}>
        <span className={styles.muted}>
          {t(
            html ? 'pluginRuntime.product.keepActive' : 'pluginRuntime.product.noConfig',
          )}
        </span>
        {draft?.status === 'generating' ? (
          <Button onClick={() => void stop()}>
            {t('pluginRuntime.product.stop')}
          </Button>
        ) : (
          <Button
            type='primary'
            icon={<Send size={15} />}
            loading={busy}
            disabled={!input.trim() || generating}
            onClick={() => void send(input)}
          >
            {t(html ? 'pluginRuntime.product.send' : 'pluginRuntime.product.generate')}
          </Button>
        )}
      </div>
    </div>
  );
  if (!draft && !appId && !loading)
    return (
      <main className={styles.page}>
        {!embedded && (
          <header className={styles.header}>
            <Button
              type='text'
              icon={<ArrowLeft />}
              onClick={() => navigate('/plugins')}
            >
              {t('pluginRuntime.product.back')}
            </Button>
            <h1>{t('pluginRuntime.product.create')}</h1>
          </header>
        )}
        <section className={styles.content}>
          <div className={styles.hero}>
            <h1>{t('pluginRuntime.product.hero')}</h1>
            <p>{t('pluginRuntime.product.heroHint')}</p>
            {modelNotice}
            {errorNotice}
            {composer}
            <div className={styles.examples}>
              {(['todo', 'timer', 'travel'] as const).map((key) => (
                <Button
                  key={key}
                  type='secondary'
                  onClick={() =>
                    setInput(t(`pluginRuntime.product.examples.${key}`))
                  }
                >
                  {t(`pluginRuntime.product.exampleNames.${key}`)}
                </Button>
              ))}
            </div>
          </div>
        </section>
      </main>
    );
  const preview = (
    <>
      <div className={styles.previewHeader}>
        <strong>{t('pluginRuntime.product.preview')}</strong>
        <span className={styles.muted}>
          {t('pluginRuntime.product.previewTemporary')}
        </span>
        <Button
          className={styles.end}
          type='text'
          icon={<PreviewOpen />}
          onClick={() => setExpanded(!expanded)}
        >
          {t(
            expanded
              ? 'pluginRuntime.product.continueEditing'
              : 'pluginRuntime.product.expand',
          )}
        </Button>
      </div>
      {loading ? (
        <div className={styles.empty}>
          <Spin />
        </div>
      ) : html ? (
        <PluginRuntimeDraftPreview
          html={previewError && lastGood.current ? lastGood.current : html}
          title={draft?.name || initial.name || t('pluginRuntime.product.preview')}
          onStatus={onPreviewStatus}
        />
      ) : draft?.service_source && !generating ? (
        <div className={styles.empty}>
          <h3>{draft.name}</h3>
          <p>{t('pluginRuntime.product.backgroundPreview')}</p>
          {draft.source_manifest?.actions?.map((action) => (
            <div key={action.id}><strong>{action.name}</strong><p>{action.description}</p></div>
          ))}
        </div>
      ) : (
        <div className={styles.empty}>
          {generating && <Spin />}
          <h3>
            {t(
              generating
                ? 'pluginRuntime.product.generating'
                : 'pluginRuntime.product.waiting',
            )}
          </h3>
          <p>{t('pluginRuntime.product.previewHint')}</p>
        </div>
      )}
    </>
  );
  return (
    <main className={styles.page}>
      <header className={styles.header}>
        <Button
          type='text'
          icon={<ArrowLeft />}
          onClick={() => navigate('/plugins')}
        >
          {t('pluginRuntime.product.back')}
        </Button>
        <h1>{draft?.name || initial.name || t('pluginRuntime.product.untitled')}</h1>
        <span className={styles.muted} role='status'>
          {t(
            generating
              ? 'pluginRuntime.product.generating'
              : 'pluginRuntime.product.draftSaved',
          )}
        </span>
        <Button
          type='primary'
          loading={busy}
          disabled={
            !draft || generating || (html ? !previewValid || Boolean(previewError) : !draft.service_source)
          }
          onClick={() => void save()}
        >
          {t(
            draft?.plugin_id
              ? 'pluginRuntime.product.saveUpdate'
              : 'pluginRuntime.product.saveOpen',
          )}
        </Button>
      </header>
      {modelNotice}
      {errorNotice}
      {previewError && (
        <Alert
          type='warning'
          content={t(
            lastGood.current
              ? 'pluginRuntime.product.previousPreview'
              : 'pluginRuntime.product.previewError',
          )}
          action={
            <Button
              disabled={generating}
              onClick={() =>
                void send(
                  t('pluginRuntime.product.repairPrompt', { error: previewError }),
                  true,
                )
              }
            >
              {t('pluginRuntime.product.repair')}
            </Button>
          }
        />
      )}
      {expanded ? (
        <section className={styles.previewOnly}>{preview}</section>
      ) : (
        <div className={styles.editor}>
          <section
            className={styles.chat}
            aria-label={t('pluginRuntime.product.chat')}
          >
            <div className={styles.messages}>
              {draft?.messages.map((message, index) => (
                <div
                  key={index}
                  className={`${styles.message} ${message.role === 'user' ? styles.user : ''}`}
                >
                  {message.role === 'assistant' && (
                    <strong>
                      Nomi
                      <br />
                    </strong>
                  )}
                  {message.content}
                </div>
              ))}
              {generating && (
                <p role='status' className={styles.muted}>
                  <Spin size={12} /> {t('pluginRuntime.product.generatingHint')}
                </p>
              )}
            </div>
            {composer}
          </section>
          <section className={styles.preview}>{preview}</section>
        </div>
      )}
    </main>
  );
}
