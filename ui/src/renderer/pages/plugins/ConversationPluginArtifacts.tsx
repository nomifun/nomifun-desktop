import { useCallback, useEffect, useRef, useState } from 'react';
import { Alert, Button, Checkbox, Input } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import { agentPlatform, conversation } from '@/common/adapter/ipcBridge';
import { uuidv7 } from '@/common/utils';
import { parseConversationId } from '@/common/types/ids';
import type { NativeAgentExecution } from '@/common/types/agentPlatform';
import type { PluginDraftSummary, PluginSurfaceDescriptor } from '@/common/types/pluginPlatform';
import { isDesktopShell } from '@/renderer/utils/platform';
import PluginSurfacePanel from './PluginSurfacePanel';
import { continuePluginConversation, replyToPluginConversation } from './pluginConversationResume';
import styles from './ConversationPluginArtifacts.module.css';

type Detail = Awaited<ReturnType<typeof pluginPlatform.authoring.details.invoke>>;
export default function ConversationPluginArtifacts({ conversationId }: { conversationId: string }) {
  const { t } = useTranslation();
  const [drafts, setDrafts] = useState<PluginDraftSummary[]>([]);
  const [execution, setExecution] = useState<{ conversationId: string; value: NativeAgentExecution | null } | null>(null);
  const [continuing, setContinuing] = useState(false);
  const [reply, setReply] = useState('');
  const pendingReply = useRef<{ text: string; key: string; execution: NativeAgentExecution } | null>(null);
  const [error, setError] = useState('');
  const generation = useRef(0);
  useEffect(() => { pendingReply.current = null; setReply(''); setError(''); }, [conversationId]);
  const refresh = useCallback(async () => {
    const requestGeneration = ++generation.current;
    const [result, execution] = await Promise.all([
      pluginPlatform.drafts.list.invoke(),
      agentPlatform.sessions.getExecution.invoke({ agent_session_id: conversationId }),
    ]);
    if (requestGeneration !== generation.current) return;
    setDrafts(result.drafts.filter(draft => draft.source_conversation_id === conversationId));
    setExecution({ conversationId, value: execution });
  }, [conversationId]);
  useEffect(() => {
    if (!isDesktopShell()) return;
    let active = true;
    void refresh().catch(() => undefined);
    const off = pluginPlatform.authoring.changed.on(event => {
      if (active && event.conversation_id === conversationId) void refresh().catch(() => undefined);
    });
    const update = (event: { conversation_id: string }) => {
      if (active && event.conversation_id === conversationId) void refresh().catch(() => undefined);
    };
    const offPause = conversation.turnPaused.on(update);
    const offStart = conversation.turnStarted.on(update);
    const offComplete = conversation.turnCompleted.on(update);
    return () => { active = false; generation.current += 1; off(); offPause(); offStart(); offComplete(); };
  }, [conversationId, refresh]);
  const currentExecution = execution?.conversationId === conversationId ? execution.value : null;
  const pending = currentExecution?.state === 'paused' && [
    'PLUGIN_DELIVERY_REQUIRED', 'PLUGIN_VERIFICATION_REQUIRED',
    'PLUGIN_CURRENT_CONVERSATION_PENDING',
  ].includes(currentExecution.pause?.reason ?? '');
  const visible = drafts.filter(draft => draft.source_conversation_id === conversationId);
  const continueTask = async () => {
    if (!pending || continuing) return;
    setContinuing(true); setError('');
    try {
      await continuePluginConversation({
        conversation_id: parseConversationId(conversationId),
        input: t(currentExecution?.pause?.reason === 'PLUGIN_CURRENT_CONVERSATION_PENDING'
          ? 'pluginPlatform.authoring.continueCurrentConversationPrompt'
          : 'pluginPlatform.authoring.continueTaskPrompt'),
        idempotency_key: `plugin-continue:${uuidv7()}`, plugin_delivery: {},
      });
      await refresh();
    } catch (caught) { setError(caught instanceof Error ? caught.message : String(caught)); }
    finally { setContinuing(false); }
  };
  const sendReply = async () => {
    const text = reply.trim();
    if (!pending || !currentExecution || !text || continuing) return;
    const operation = pendingReply.current?.text === text ? pendingReply.current : {
      text, key: `plugin-reply:${uuidv7()}`, execution: currentExecution,
    };
    pendingReply.current = operation;
    setContinuing(true); setError('');
    try {
      await replyToPluginConversation({
        conversation_id: parseConversationId(conversationId), input: text, idempotency_key: operation.key,
      }, operation.execution);
      pendingReply.current = null; setReply('');
      await refresh();
    } catch (caught) { setError(caught instanceof Error ? caught.message : String(caught)); }
    finally { setContinuing(false); }
  };
  if ((!visible.length && !pending) || !isDesktopShell()) return null;
  return <div className={styles.list}>
    {pending && <Alert type='warning' content={<div>
      <p>{t(currentExecution?.pause?.reason === 'PLUGIN_CURRENT_CONVERSATION_PENDING'
        ? 'pluginPlatform.authoring.currentConversationPending' : 'pluginPlatform.authoring.deliveryPending')}</p>
      <Button loading={continuing} onClick={() => void continueTask()}>{t('pluginPlatform.authoring.resumeTask')}</Button>
      <div className={styles.reply}>
        <label htmlFor={`plugin-reply-${conversationId}`}>{t('pluginPlatform.authoring.replyLabel')}</label>
        <Input.TextArea id={`plugin-reply-${conversationId}`} value={reply} onChange={setReply} disabled={continuing}
          placeholder={t('pluginPlatform.authoring.replyPlaceholder')} autoSize={{ minRows: 2, maxRows: 4 }} />
        <Button type='primary' disabled={!reply.trim()} loading={continuing} onClick={() => void sendReply()}>
          {t('pluginPlatform.authoring.replyContinue')}
        </Button>
      </div>
    </div>} />}
    {error && <Alert type='error' content={error} />}
    {visible.map(draft => <Artifact key={draft.draft_id} summary={draft} refresh={refresh} />)}
  </div>;
}

function Artifact({ summary, refresh }: { summary: PluginDraftSummary; refresh: () => Promise<void> }) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [detail, setDetail] = useState<Detail | null>(null);
  const [accepted, setAccepted] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [installedSurface, setInstalledSurface] = useState<PluginSurfaceDescriptor | null>(null);
  const installedRef = useRef<PluginSurfaceDescriptor | null>(null);
  const [previewClosed, setPreviewClosed] = useState(false);
  const [reloadKey, setReloadKey] = useState(0);
  const loadGeneration = useRef(0);
  const load = useCallback(async () => {
    const generation = ++loadGeneration.current;
    const next = await pluginPlatform.authoring.details.invoke({ draft_id: summary.draft_id });
    if (generation === loadGeneration.current) setDetail(next);
    return next;
  }, [summary.draft_id]);
  useEffect(() => {
    let active = true;
    void load().catch(() => undefined);
    const off = pluginPlatform.authoring.changed.on(event => {
      if (active && event.draft_id === summary.draft_id) void load().catch(() => undefined);
    });
    return () => { active = false; loadGeneration.current += 1; off(); };
  }, [summary.draft_id, summary.revision, load]);
  useEffect(() => () => {
    const descriptor = installedRef.current;
    if (descriptor) void pluginPlatform.surface.close.invoke({
      plugin_id: descriptor.plugin_id, is_preview: false,
      request: { surface_session_id: descriptor.surface_session_id, surface_generation: descriptor.surface_generation },
    }).catch(() => undefined);
  }, []);
  const delivered = Boolean(detail?.verification.delivery);
  const savedId = delivered ? detail?.draft.summary.plugin_id : undefined;
  const descriptor = savedId ? installedSurface : previewClosed ? undefined
    : detail?.verification.surface as PluginSurfaceDescriptor | undefined;
  useEffect(() => { setPreviewClosed(false); }, [
    (detail?.verification.surface as PluginSurfaceDescriptor | undefined)?.surface_session_id,
    (detail?.verification.surface as PluginSurfaceDescriptor | undefined)?.surface_generation,
  ]);
  const command = detail?.commands[0];
  const permission = detail?.confirmation;
  useEffect(() => { setAccepted(false); }, [permission?.confirmation_id]);
  const approve = async () => {
    if (!permission || !detail || !accepted || busy) return;
    setBusy(true); setError('');
    try {
      await pluginPlatform.authoring.approve.invoke({
        draft_id: summary.draft_id,
        request: { expected_revision: detail.draft.summary.revision, confirmation_id: permission.confirmation_id, approved: true },
      });
      await load(); await refresh();
      const conversationId = detail.draft.summary.source_conversation_id;
      if (conversationId) {
        const message = {
          conversation_id: parseConversationId(conversationId),
          input: t('pluginPlatform.authoring.resumePrompt', { id: summary.draft_id }),
          idempotency_key: `plugin-approval:${permission.confirmation_id}`,
          plugin_delivery: { draft_id: summary.draft_id },
        };
        await continuePluginConversation(message);
      }
    } catch (caught) { setError(caught instanceof Error ? caught.message : String(caught)); }
    finally { setBusy(false); }
  };
  const open = async () => {
    if (!savedId || busy) return;
    setBusy(true);
    try {
      const plugin = await pluginPlatform.plugins.get.invoke({ plugin_id: savedId });
      if (!plugin.summary.has_ui) { navigate(`/plugins/run/${encodeURIComponent(savedId)}`); return; }
      const previous = installedRef.current;
      if (previous) await pluginPlatform.surface.close.invoke({
        plugin_id: previous.plugin_id, is_preview: false,
        request: { surface_session_id: previous.surface_session_id, surface_generation: previous.surface_generation },
      }).catch(() => undefined);
      const current = await pluginPlatform.plugins.openSurface.invoke({
        plugin_id: savedId, request: { expected_revision: plugin.summary.revision },
      });
      installedRef.current = current;
      setInstalledSurface(current);
    } catch (caught) { setError(caught instanceof Error ? caught.message : String(caught)); }
    finally { setBusy(false); }
  };
  return <section className={styles.card}>
    <div className={styles.header}>
      <strong>{summary.display_name}</strong>
      <span>{t(savedId ? 'pluginPlatform.authoring.saved' : delivered
        ? 'pluginPlatform.authoring.historicalDeleted' : 'pluginPlatform.authoring.inProgress')}</span>
      {savedId && <Button size='small' loading={busy} onClick={() => void open()}>{t('pluginPlatform.actions.open')}</Button>}
    </div>
    {error && <Alert type='error' content={error} />}
    {permission && <div className={styles.approval}>
      <p>{t('pluginPlatform.authoring.approvalHint')}</p>
      {permission.trusted_local_service && <p>{t('pluginPlatform.permissions.localCode')}</p>}
      {permission.added_permissions.length > 0 && <p>{permission.added_permissions.join(', ')}</p>}
      {permission.added_secret_slots.length > 0 && <p>{permission.added_secret_slots.join(', ')}</p>}
      <Checkbox checked={accepted} onChange={setAccepted}>{t('pluginPlatform.authoring.accept')}</Checkbox>
      <Button type='primary' disabled={!accepted} loading={busy} onClick={() => void approve()}>{t('pluginPlatform.authoring.approve')}</Button>
    </div>}
    {descriptor?.entrypoint && <div className={styles.preview}>
      <PluginSurfacePanel key={reloadKey} descriptor={descriptor} title={summary.display_name}
        onReload={() => savedId ? void open() : setReloadKey(previous => previous + 1)}
        onClose={async () => {
          await pluginPlatform.surface.close.invoke({
            plugin_id: descriptor.plugin_id, draft_id: descriptor.draft_id, is_preview: descriptor.is_preview,
            request: { surface_session_id: descriptor.surface_session_id, surface_generation: descriptor.surface_generation },
          });
          setInstalledSurface(null); installedRef.current = null; setPreviewClosed(true);
        }}
        verification={command && !savedId ? {
          command,
          onComplete: async (observations, failure) => {
            await pluginPlatform.authoring.uiResults.invoke({
              draft_id: summary.draft_id,
              request: { test_token: command.test_token, descriptor: command.descriptor, observations, ...(failure ? { error: failure } : {}) },
            });
          },
        } : undefined}
      />
    </div>}
    {detail && <details className={styles.details}>
      <summary>{t('pluginPlatform.authoring.details')}</summary>
      {Object.keys(detail.draft.imported_context).length > 0 && <div>
        <p>{t('pluginPlatform.authoring.importedHistoryHint')}</p>
        <pre>{JSON.stringify(detail.draft.imported_context, null, 2)}</pre>
      </div>}
      <pre>{JSON.stringify(detail.verification, null, 2)}</pre>
      {detail.draft.files.map(file => <details key={file.path}>
        <summary>{file.path}</summary><pre>{file.text ?? file.media_type}</pre>
      </details>)}
    </details>}
  </section>;
}
