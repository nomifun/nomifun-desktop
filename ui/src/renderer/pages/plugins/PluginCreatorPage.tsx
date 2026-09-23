import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Alert, Button, Checkbox, Input, Modal, Select, Spin } from '@arco-design/web-react';
import {
  ArrowLeft,
  CheckOne,
  Code,
  EditTwo,
  Info,
  Loading,
  Magic,
  PreviewOpen,
  Robot,
  Save,
  Send,
  SettingTwo,
  Shield,
  User,
} from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type {
  PluginDraftDetail,
  PluginCredentialReference,
  PluginManifestSummary,
  PluginPermissionExpansion,
  PluginSurfaceDescriptor,
} from '@/common/types/pluginPlatform';
import { useGuidModelSelection } from '@/renderer/pages/guid/hooks/useGuidModelSelection';
import { isDesktopShell } from '@/renderer/utils/platform';
import { notifyPluginLibraryChanged } from './pluginLibraryState';
import { draftManifest } from './pluginPlatformModel';
import PluginSurfacePanel from './PluginSurfacePanel';
import PluginWorkspace, { PluginVisual } from './PluginWorkspace';
import styles from './PluginPlatform.module.css';

function textBase64(value: string): string {
  const bytes = new TextEncoder().encode(value);
  let binary = '';
  for (let index = 0; index < bytes.length; index += 1) binary += String.fromCharCode(bytes[index]!);
  return btoa(binary);
}

export default function PluginCreatorPage() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { draftId } = useParams<{ draftId: string }>();
  const [searchParams] = useSearchParams();
  const { current_model } = useGuidModelSelection('nomi');
  const desktopShell = isDesktopShell();
  const [draft, setDraft] = useState<PluginDraftDetail | null>(null);
  const [input, setInput] = useState(searchParams.get('requirement') ?? '');
  const [selectedPath, setSelectedPath] = useState('nomifun.plugin.json');
  const [editor, setEditor] = useState('');
  const [preview, setPreview] = useState<PluginSurfaceDescriptor | null>(null);
  const [draftConfig, setDraftConfig] = useState('{}');
  const [previewPermissions, setPreviewPermissions] = useState<string[]>([]);
  const [previewCredentials, setPreviewCredentials] = useState<Record<string, string>>({});
  const [credentialOptions, setCredentialOptions] = useState<PluginCredentialReference[]>([]);
  const [confirmation, setConfirmation] = useState<PluginPermissionExpansion | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(Boolean(draftId));
  const [error, setError] = useState('');
  const polling = useRef<number | undefined>(undefined);
  const ownedGenerationDraft = useRef<string | null>(null);

  const manifest = useMemo(() => draftManifest(draft?.files ?? []), [draft?.files]);
  const selectedFile = draft?.files.find((file) => file.path === selectedPath);

  useEffect(() => {
    setEditor(selectedFile?.text ?? '');
  }, [selectedFile?.digest, selectedFile?.text]);

  useEffect(() => {
    if (!desktopShell) return;
    let active = true;
    void pluginPlatform.credentials.list.invoke().then((references) => {
      if (active) setCredentialOptions(references);
    }).catch(() => {
      if (active) setCredentialOptions([]);
    });
    return () => { active = false; };
  }, [desktopShell]);

  const load = useCallback(async (id: string) => {
    setLoading(true);
    try {
      const loaded = await pluginPlatform.drafts.get.invoke({ draft_id: id });
      setDraft(loaded);
      if (loaded.summary.plugin_id) {
        const detail = await pluginPlatform.plugins.get.invoke({
          plugin_id: loaded.summary.plugin_id,
        });
        setDraftConfig(JSON.stringify(detail.config.values, null, 2));
        setPreviewCredentials(Object.fromEntries(detail.manifest.secret_slots.map((slot) => [
          slot,
          detail.credential_bindings.find((binding) => binding.slot === slot)?.credential_id ?? '',
        ])));
        setPreviewPermissions(detail.grants
          .filter((grant) => grant.granted)
          .map((grant) => grant.permission));
      } else {
        setDraftConfig('{}');
        setPreviewCredentials({});
        setPreviewPermissions([]);
      }
      setError('');
    } catch (caught) {
      console.error('[pluginPlatform] Draft load failed', caught);
      setError(t('pluginPlatform.creator.loadFailed'));
    } finally {
      setLoading(false);
    }
  }, [t]);

  useEffect(() => {
    if (draftId) void load(draftId);
  }, [draftId, load]);

  const closePreview = useCallback(async () => {
    if (!desktopShell) return;
    if (!preview) return;
    const closing = preview;
    setPreview(null);
    try {
      await pluginPlatform.surface.close.invoke({
        plugin_id: closing.plugin_id,
        draft_id: closing.draft_id,
        is_preview: closing.is_preview,
        request: {
          surface_session_id: closing.surface_session_id,
          surface_generation: closing.surface_generation,
        },
      });
    } catch {
      // Session expiry and activation revoke the same authority; close is best effort.
    }
  }, [desktopShell, preview]);

  const parseDraftConfig = useCallback((): Record<string, unknown> | null => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(draftConfig);
    } catch {
      setError(t('pluginPlatform.config.invalidJson'));
      return null;
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      setError(t('pluginPlatform.config.objectRequired'));
      return null;
    }
    return parsed as Record<string, unknown>;
  }, [draftConfig, t]);

  const selectedCredentialBindings = useCallback((targetManifest: PluginManifestSummary) => (
    Object.fromEntries(targetManifest.secret_slots.flatMap((slot) => {
      const credentialId = previewCredentials[slot]?.trim();
      return credentialId ? [[slot, credentialId]] : [];
    }))
  ), [previewCredentials]);

  const unavailableCredentialSlot = useCallback((bindings: Record<string, string>) => (
    Object.entries(bindings).find(([, credentialId]) => (
      !credentialOptions.some((reference) => (
        reference.credential_id === credentialId && reference.enabled
      ))
    ))?.[0]
  ), [credentialOptions]);

  const reloadPreview = useCallback(async (target: PluginDraftDetail): Promise<boolean> => {
    if (!desktopShell) return false;
    const targetManifest = draftManifest(target.files);
    if (!targetManifest?.entrypoints.ui) {
      await closePreview();
      return false;
    }
    const config = parseDraftConfig();
    if (!config) return false;
    const credentialBindings = selectedCredentialBindings(targetManifest);
    const unavailableSlot = unavailableCredentialSlot(credentialBindings);
    if (unavailableSlot) {
      setError(t('pluginPlatform.preview.credentialUnavailable', { slot: unavailableSlot }));
      return false;
    }
    const declaredPermissions = new Set(targetManifest.permissions);
    const response = await pluginPlatform.drafts.preview.invoke({
      draft_id: target.summary.draft_id,
      request: {
        expected_revision: target.summary.revision,
        config,
        access: {
          permissions: previewPermissions.filter((permission) => declaredPermissions.has(permission)),
          credential_bindings: credentialBindings,
        },
      },
    });
    setPreview(response.descriptor);
    setDraft({
      ...target,
      summary: { ...target.summary, revision: response.draft_revision },
    });
    return true;
  }, [closePreview, desktopShell, parseDraftConfig, previewPermissions, selectedCredentialBindings, t, unavailableCredentialSlot]);

  useEffect(() => {
    window.clearTimeout(polling.current);
    if (!draft || draft.summary.status !== 'generating') return;
    if (ownedGenerationDraft.current === draft.summary.draft_id) return;
    const poll = async () => {
      try {
        const next = await pluginPlatform.drafts.get.invoke({ draft_id: draft.summary.draft_id });
        setDraft(next);
        if (next.summary.status === 'generating') {
          polling.current = window.setTimeout(poll, 1200);
        } else {
          notifyPluginLibraryChanged();
          if (next.summary.status === 'ready') {
            try {
              await reloadPreview(next);
            } catch (caught) {
              console.error('[pluginPlatform] automatic Preview reload failed', caught);
              setError(t('pluginPlatform.preview.failed'));
            }
          }
        }
      } catch {
        polling.current = window.setTimeout(poll, 2000);
      }
    };
    polling.current = window.setTimeout(poll, 1200);
    return () => window.clearTimeout(polling.current);
  }, [draft?.summary.draft_id, draft?.summary.revision, draft?.summary.status, reloadPreview, t]);

  useEffect(() => () => {
    if (desktopShell && preview) void pluginPlatform.surface.close.invoke({
      plugin_id: preview.plugin_id,
      draft_id: preview.draft_id,
      is_preview: preview.is_preview,
      request: {
        surface_session_id: preview.surface_session_id,
        surface_generation: preview.surface_generation,
      },
    }).catch(() => undefined);
  }, [desktopShell, preview]);

  const send = async () => {
    if (!desktopShell) return;
    const requirement = input.trim();
    if (!requirement || busy) return;
    if (!current_model) {
      setError(t('pluginPlatform.creator.modelRequired'));
      return;
    }
    setBusy(true);
    setError('');
    let generationDraftId: string | undefined;
    try {
      let current = draft;
      if (!current) {
        current = await pluginPlatform.drafts.create.invoke({});
        setDraft(current);
        setDraftConfig('{}');
        setPreviewCredentials({});
        setPreviewPermissions([]);
        navigate(`/plugins/create/${encodeURIComponent(current.summary.draft_id)}`, { replace: true });
      }
      generationDraftId = current.summary.draft_id;
      ownedGenerationDraft.current = current.summary.draft_id;
      const generation = pluginPlatform.drafts.generate.invoke({
        draft_id: current.summary.draft_id,
        request: {
          expected_revision: current.summary.revision,
          provider_id: String(current_model.id),
          model: current_model.use_model,
          requirement,
        },
      });
      setDraft({
        ...current,
        summary: {
          ...current.summary,
          revision: current.summary.revision + 1,
          status: 'generating',
          error_code: undefined,
        },
      });
      setBusy(false);
      const next = await generation;
      ownedGenerationDraft.current = null;
      setBusy(true);
      setDraft(next);
      setInput('');
      notifyPluginLibraryChanged();
      try {
        await reloadPreview(next);
      } catch (caught) {
        console.error('[pluginPlatform] automatic Preview reload failed', caught);
        setError(t('pluginPlatform.preview.failed'));
      }
    } catch (caught) {
      ownedGenerationDraft.current = null;
      console.error('[pluginPlatform] generation failed', caught);
      setError(t('pluginPlatform.creator.generateFailed'));
      if (generationDraftId) void load(generationDraftId);
    } finally {
      ownedGenerationDraft.current = null;
      setBusy(false);
    }
  };

  const cancelGeneration = async () => {
    if (!desktopShell || !draft || busy) return;
    setBusy(true);
    try {
      setDraft(await pluginPlatform.drafts.cancelGeneration.invoke({
        draft_id: draft.summary.draft_id,
        request: { expected_revision: draft.summary.revision },
      }));
    } catch {
      setError(t('pluginPlatform.creator.operationFailed'));
    } finally {
      setBusy(false);
    }
  };

  const saveFile = async () => {
    if (!desktopShell || !draft || !selectedFile?.text || editor === selectedFile.text || busy) return;
    setBusy(true);
    try {
      const next = await pluginPlatform.drafts.replaceFile.invoke({
        draft_id: draft.summary.draft_id,
        request: {
          expected_revision: draft.summary.revision,
          path: selectedFile.path,
          content_base64: textBase64(editor),
        },
      });
      setDraft(next);
      try {
        await reloadPreview(next);
      } catch (caught) {
        console.error('[pluginPlatform] automatic Preview reload failed', caught);
        setError(t('pluginPlatform.preview.failed'));
      }
    } catch {
      setError(t('pluginPlatform.creator.fileSaveFailed'));
    } finally {
      setBusy(false);
    }
  };

  const openPreview = async () => {
    if (!desktopShell || !draft || !manifest?.entrypoints.ui || busy) return;
    setBusy(true);
    setError('');
    try {
      await reloadPreview(draft);
    } catch (caught) {
      console.error('[pluginPlatform] preview failed', caught);
      setError(t('pluginPlatform.preview.failed'));
    } finally {
      setBusy(false);
    }
  };

  const save = async (permissionConfirmationId?: string) => {
    if (!desktopShell || !draft || !manifest || busy || (manifest.entrypoints.ui && !preview)) return;
    const config = parseDraftConfig();
    if (!config) return;
    const credentialBindings = selectedCredentialBindings(manifest);
    const unavailableSlot = unavailableCredentialSlot(credentialBindings);
    if (unavailableSlot) {
      setError(t('pluginPlatform.config.credentialUnavailableSelected', {
        slot: unavailableSlot,
      }));
      return;
    }
    setBusy(true);
    setError('');
    try {
      const response = await pluginPlatform.drafts.save.invoke({
        draft_id: draft.summary.draft_id,
        request: {
          expected_revision: draft.summary.revision,
          ...(draft.summary.base_plugin_revision === undefined
            ? {}
            : { expected_plugin_revision: draft.summary.base_plugin_revision }),
          ...(permissionConfirmationId
            ? { permission_confirmation_id: permissionConfirmationId }
            : {}),
          config,
          credential_bindings: credentialBindings,
        },
      });
      setDraft((current) => current ? { ...current, summary: response.draft } : current);
      if (response.result.outcome === 'confirmation_required') {
        setConfirmation(response.result.confirmation);
        return;
      }
      setConfirmation(null);
      await closePreview();
      notifyPluginLibraryChanged();
      navigate(`/plugins/run/${encodeURIComponent(response.result.plugin.summary.plugin_id)}?saved=1`);
    } catch (caught) {
      console.error('[pluginPlatform] Draft save failed', caught);
      setError(t('pluginPlatform.creator.saveFailed'));
      if (draftId) void load(draftId);
    } finally {
      setBusy(false);
    }
  };

  const removeDraft = async () => {
    if (!desktopShell || !draft || busy) return;
    setBusy(true);
    try {
      await closePreview();
      await pluginPlatform.drafts.delete.invoke({
        draft_id: draft.summary.draft_id,
        request: { expected_revision: draft.summary.revision },
      });
      notifyPluginLibraryChanged();
      navigate('/plugins');
    } catch {
      setError(t('pluginPlatform.creator.operationFailed'));
    } finally {
      setBusy(false);
    }
  };

  const generating = draft?.summary.status === 'generating';
  const canSave = Boolean(
    desktopShell && draft && manifest && !generating && (!manifest.entrypoints.ui || preview),
  );
  const requirementUnderstood = Boolean(draft?.messages.length || draft);
  const implementationReady = Boolean(manifest && !generating);
  const previewReady = Boolean(preview || (manifest && !manifest.entrypoints.ui));
  const buildProgress = previewReady ? 100 : generating ? 68 : implementationReady ? 84 : requirementUnderstood ? 34 : 8;

  if (loading) return (
    <PluginWorkspace activeView='drafts'>
      <main className={styles.page}><div className={styles.emptyState}><Spin /></div></main>
    </PluginWorkspace>
  );

  return (
    <PluginWorkspace activeView='drafts'>
      <main className={styles.page}>
        <div className={styles.breadcrumb}>
          <button type='button' onClick={() => navigate('/plugins')}>
            <ArrowLeft />{t('pluginPlatform.workspace.title')}
          </button>
          <span>/</span>
          <span>{t('pluginPlatform.workspace.views.drafts')}</span>
        </div>

        <header className={styles.creatorHeader}>
          <div className={styles.creatorIdentity}>
            <PluginVisual draft large />
            <div className={styles.headerCopy}>
              <div className={styles.pluginTitleLine}>
                <h1>{draft?.summary.display_name || t('pluginPlatform.creator.title')}</h1>
                <span className={styles.statusPill} data-tone={generating ? 'attention' : 'draft'}>
                  {draft ? t(`pluginPlatform.creator.status.${draft.summary.status}`) : t('pluginPlatform.creator.status.new')}
                </span>
              </div>
              <p>{t('pluginPlatform.creator.subtitle')}</p>
            </div>
          </div>
          <div className={styles.headerActions}>
            {desktopShell && draft && <Button disabled={busy || generating} onClick={() => void removeDraft()}>
              {t('pluginPlatform.creator.discard')}
            </Button>}
            {desktopShell && <Button
              type='primary'
              icon={<Save />}
              loading={busy}
              disabled={!canSave}
              onClick={() => void save()}
            >
              {t('pluginPlatform.creator.save')}
            </Button>}
          </div>
        </header>

        {error && <Alert type='error' content={error} showIcon />}
        {!desktopShell && <Alert type='info' content={t('pluginPlatform.readOnly.body')} showIcon />}
        {desktopShell && !current_model && <Alert type='warning' content={t('pluginPlatform.creator.modelRequired')} showIcon />}
        {manifest?.entrypoints.service && <Alert type='warning' content={t('pluginPlatform.permissions.localCode')} showIcon />}

        <div className={styles.creatorStage}>
          <section className={`${styles.section} ${styles.creatorConversation}`}>
            <div className={styles.sectionTitleRow}>
              <div className={styles.creatorPanelTitle}>
                <span className={styles.sectionHeadingIcon}><Robot /></span>
                <div><h2>{t('pluginPlatform.creator.assistantTitle')}</h2><p>{t('pluginPlatform.creator.assistantHelp')}</p></div>
              </div>
              <span className={styles.statusPill} data-tone={current_model ? 'enabled' : 'disabled'}>
                {current_model ? t('pluginPlatform.creator.modelConnected') : t('pluginPlatform.creator.modelDisconnected')}
              </span>
            </div>
            <div className={styles.messageList}>
              {!draft?.messages.length && !generating && (
                <div className={styles.creatorWelcome}>
                  <span><Magic /></span>
                  <h3>{t('pluginPlatform.creator.welcomeTitle')}</h3>
                  <p>{t('pluginPlatform.creator.welcomeBody')}</p>
                </div>
              )}
              {draft?.messages.map((message, index) => (
                <div
                  key={`${message.role}:${index}`}
                  className={`${styles.messageTurn} ${message.role === 'user' ? styles.messageTurnUser : ''}`}
                >
                  <span className={styles.messageAvatar}>{message.role === 'user' ? <User /> : <Robot />}</span>
                  <div>
                    <strong>{message.role === 'user' ? t('pluginPlatform.creator.you') : t('pluginPlatform.creator.assistant')}</strong>
                    <p>{message.content}</p>
                  </div>
                </div>
              ))}
              {generating && (
                <div className={styles.messageTurn}>
                  <span className={`${styles.messageAvatar} ${styles.messageAvatarBusy}`}><Loading /></span>
                  <div><strong>{t('pluginPlatform.creator.assistant')}</strong><p>{t('pluginPlatform.creator.generating')}</p></div>
                </div>
              )}
            </div>
            {desktopShell && <div className={styles.composer}>
              <Input.TextArea
                value={input}
                onChange={setInput}
                disabled={generating || busy}
                autoSize={{ minRows: 2, maxRows: 7 }}
                placeholder={t('pluginPlatform.creator.placeholder')}
                onPressEnter={(event) => {
                  if (!event.shiftKey && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    void send();
                  }
                }}
              />
              <div className={styles.composerActions}>
                <span>{t('pluginPlatform.creator.composerHint')}</span>
                {generating ? (
                  <Button onClick={() => void cancelGeneration()}>{t('pluginPlatform.creator.stop')}</Button>
                ) : (
                  <Button type='primary' icon={<Send />} disabled={!input.trim() || busy} onClick={() => void send()}>
                    {t('pluginPlatform.creator.send')}
                  </Button>
                )}
              </div>
            </div>}
          </section>

          <aside className={styles.creatorRail}>
            <section className={styles.section}>
              <div className={styles.sectionTitleRow}>
                <div><h2>{t('pluginPlatform.creator.buildStatus')}</h2><p>{t('pluginPlatform.creator.buildStatusHelp')}</p></div>
                <strong className={styles.progressValue}>{buildProgress}%</strong>
              </div>
              <div className={styles.progressTrack}><span style={{ width: `${buildProgress}%` }} /></div>
              <div className={styles.buildSteps}>
                <BuildStep done={requirementUnderstood} active={!requirementUnderstood} icon={<Magic />} label={t('pluginPlatform.creator.steps.requirement')} />
                <BuildStep done={implementationReady} active={generating || (requirementUnderstood && !implementationReady)} icon={<Code />} label={t('pluginPlatform.creator.steps.implementation')} />
                <BuildStep done={previewReady} active={implementationReady && !previewReady} icon={<PreviewOpen />} label={t('pluginPlatform.creator.steps.preview')} />
              </div>
            </section>

            <section className={styles.section}>
              <div className={styles.sectionTitleRow}>
                <div><h2>{t('pluginPlatform.preview.title')}</h2><p>{t('pluginPlatform.preview.temporaryShort')}</p></div>
                <PreviewOpen />
              </div>
              {manifest ? (
                <div className={styles.previewSummary}>
                  <div className={styles.previewPluginIdentity}>
                    <PluginVisual shape={manifest.entrypoints.ui && manifest.entrypoints.service ? 'mixed' : manifest.entrypoints.ui ? 'ui_only' : 'headless'} />
                    <span><strong>{manifest.name}</strong><small>{manifest.package_id} · {manifest.version}</small></span>
                  </div>
                  <div className={styles.previewFacts}>
                    <span><Code />{t('pluginPlatform.creator.capabilityCount', { count: manifest.actions.length })}</span>
                    <span><Shield />{t('pluginPlatform.creator.permissionCount', { count: manifest.permissions.length })}</span>
                  </div>
                  {manifest.entrypoints.ui ? (
                    desktopShell && <Button type='primary' long icon={<PreviewOpen />} loading={busy} disabled={generating} onClick={() => void openPreview()}>
                      {preview ? t('pluginPlatform.preview.reload') : t('pluginPlatform.preview.open')}
                    </Button>
                  ) : (
                    <div className={styles.previewNote}><Info />{t('pluginPlatform.creator.headlessPreview')}</div>
                  )}
                </div>
              ) : (
                <div className={styles.previewPlaceholder}>
                  <PluginVisual draft />
                  <p>{t('pluginPlatform.creator.previewWaiting')}</p>
                </div>
              )}
              {manifest?.permissions.length ? (
                <details className={styles.permissionDisclosure}>
                  <summary>{t('pluginPlatform.preview.permissions')}</summary>
                  {manifest.permissions.map((permission) => (
                    <Checkbox
                      key={permission}
                      disabled={!desktopShell}
                      checked={previewPermissions.includes(permission)}
                      onChange={(checked) => setPreviewPermissions((current) => checked
                        ? [...new Set([...current, permission])]
                        : current.filter((value) => value !== permission))}
                    >{permission}</Checkbox>
                  ))}
                </details>
              ) : null}
            </section>
          </aside>
        </div>

        {manifest?.entrypoints.ui && preview && desktopShell && (
          <section className={styles.surfaceSection}>
            <div className={styles.sectionTitleRow}>
              <div><h2>{t('pluginPlatform.creator.livePreview')}</h2><p>{t('pluginPlatform.preview.temporary')}</p></div>
            </div>
            <PluginSurfacePanel
              descriptor={preview}
              title={draft?.summary.display_name || t('pluginPlatform.preview.title')}
              closing={busy}
              onReload={() => void openPreview()}
              onClose={() => void closePreview()}
            />
          </section>
        )}

        <details className={styles.developerSection}>
          <summary>
            <span><EditTwo />{t('pluginPlatform.creator.files')}</span>
            {manifest && <span className={styles.kindBadge} data-tone='draft'>{manifest.package_id} · {manifest.version}</span>}
          </summary>
          <div className={styles.developerBody}>
            <div className={styles.packageEditorLayout}>
              <div className={styles.fileList}>
                {draft?.files.map((file) => (
                  <button
                    type='button'
                    key={file.path}
                    className={`${styles.fileButton} ${selectedPath === file.path ? styles.fileButtonActive : ''}`}
                    onClick={() => setSelectedPath(file.path)}
                  >
                    <span>{file.path}</span><small>{file.size_bytes} B</small>
                  </button>
                ))}
              </div>
              <div className={styles.codeEditorPane}>
                {selectedFile?.text === undefined ? (
                  <p className={styles.muted}>{t('pluginPlatform.creator.binaryFile')}</p>
                ) : (
                  <>
                    <textarea
                      className={styles.code}
                      value={editor}
                      onChange={(event) => setEditor(event.currentTarget.value)}
                      readOnly={!desktopShell}
                      spellCheck={false}
                      aria-label={selectedPath}
                    />
                    {desktopShell && <Button disabled={editor === selectedFile.text || busy || generating} onClick={() => void saveFile()}>
                      {t('pluginPlatform.creator.saveFile')}
                    </Button>}
                  </>
                )}
              </div>
            </div>
          </div>
        </details>

        {manifest && (
          <details className={styles.developerSection}>
            <summary><span><SettingTwo />{t('pluginPlatform.config.title')}</span></summary>
            <div className={`${styles.developerBody} ${styles.configEditor}`}>
              <p>{t('pluginPlatform.config.secretBoundary')}</p>
              <label>
                <strong>{t('pluginPlatform.config.values')}</strong>
                <Input.TextArea
                  className={styles.jsonEditor}
                  value={draftConfig}
                  onChange={setDraftConfig}
                  disabled={!desktopShell}
                  spellCheck={false}
                  aria-label={t('pluginPlatform.config.values')}
                />
              </label>
              <details className={styles.codeDisclosure}>
                <summary>{t('pluginPlatform.config.schema')}</summary>
                <pre>{JSON.stringify(manifest.config_schema, null, 2)}</pre>
              </details>
              {manifest.secret_slots.map((slot) => <label key={slot}>
                <strong>{slot}</strong>
                <Select
                  allowClear
                  showSearch
                  disabled={!desktopShell}
                  value={previewCredentials[slot] ?? ''}
                  onChange={(value) => setPreviewCredentials((current) => ({
                    ...current,
                    [slot]: typeof value === 'string' ? value : '',
                  }))}
                  placeholder={t('pluginPlatform.config.credentialReference')}
                  aria-label={t('pluginPlatform.config.credentialSlot', { slot })}
                  options={credentialOptions.map((reference) => ({
                    value: reference.credential_id,
                    label: reference.enabled
                      ? `${reference.label} · ${reference.kind}`
                      : `${reference.label} · ${reference.kind} (${t('pluginPlatform.config.credentialUnavailable')})`,
                    disabled: !reference.enabled,
                  }))}
                />
              </label>)}
            </div>
          </details>
        )}

        {manifest && !manifest.entrypoints.ui && (
          <section className={styles.section}>
            <div className={styles.sectionTitleRow}><h2>{t('pluginPlatform.detail.actions')}</h2><span className={styles.sectionCount}>{manifest.actions.length}</span></div>
            <div className={styles.bindingList}>
              {manifest.actions.map((action) => (
                <div key={action.action_id} className={styles.capabilityRow}>
                  <span className={styles.capabilityRowIcon}><Code /></span>
                  <span className={styles.capabilityRowCopy}><strong>{action.name}</strong><small>{action.description}</small></span>
                  <code>{action.action_id}</code>
                </div>
              ))}
            </div>
          </section>
        )}
      </main>

      <Modal
        visible={desktopShell && Boolean(confirmation)}
        title={t('pluginPlatform.permissions.title')}
        confirmLoading={busy}
        onCancel={() => setConfirmation(null)}
        onOk={() => {
          if (confirmation) void save(confirmation.confirmation_id);
        }}
        okText={t('pluginPlatform.permissions.confirm')}
      >
        {confirmation && <div className={styles.modalBody}>
          {confirmation.added_permissions.length > 0 && <Alert type='warning' content={t('pluginPlatform.permissions.added', { permissions: confirmation.added_permissions.join(', ') })} showIcon />}
          {confirmation.added_secret_slots.length > 0 && <Alert type='warning' content={t('pluginPlatform.permissions.secrets', { slots: confirmation.added_secret_slots.join(', ') })} showIcon />}
          {confirmation.trusted_local_service && <Alert type='warning' content={t('pluginPlatform.permissions.localCode')} showIcon />}
        </div>}
      </Modal>
    </PluginWorkspace>
  );
}

function BuildStep({ done, active, icon, label }: { done: boolean; active: boolean; icon: ReactNode; label: string }) {
  return (
    <div className={styles.buildStep} data-state={done ? 'done' : active ? 'active' : 'pending'}>
      <span>{done ? <CheckOne /> : icon}</span>
      <strong>{label}</strong>
    </div>
  );
}
