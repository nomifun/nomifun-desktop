import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Alert,
  Button,
  Modal,
  Spin,
  type ModalProps,
} from '@arco-design/web-react';
import { Download } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';
import {
  miniAppProduct,
  type MiniAppDraft,
} from '@/common/adapter/miniAppProductBridge';
import MiniAppDraftPreview from './MiniAppDraftPreview';
import styles from './MiniAppProduct.module.css';

const Dialog = Modal as unknown as React.ComponentType<
  React.PropsWithChildren<ModalProps>
>;
export default function MiniAppImportDialog({
  visible,
  onClose,
  onOpened,
}: {
  visible: boolean;
  onClose: () => void;
  onOpened: (id: string) => void;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<MiniAppDraft | null>(null),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(''),
    [preview, setPreview] = useState(false);
  const inspectPath = useRef<(path: string) => void>(() => undefined);
  useEffect(() => {
    if (!visible || busy || draft || !isTauriRuntime()) return;
    let stopped = false;
    let unlisten: (() => void) | undefined;
    void import('@tauri-apps/api/webview')
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((event) => {
          if (
            !stopped &&
            event.payload.type === 'drop' &&
            event.payload.paths[0]
          )
            inspectPath.current(event.payload.paths[0]);
        }),
      )
      .then((stop) => {
        if (stopped) stop();
        else unlisten = stop;
      })
      .catch(() => undefined);
    return () => {
      stopped = true;
      unlisten?.();
    };
  }, [visible, busy, draft]);
  useEffect(() => {
    if (visible) {
      setDraft(null);
      setError('');
      setPreview(false);
    }
  }, [visible]);
  const inspect = async (request: {
    source_path?: string;
    filename?: string;
    content?: string;
  }) => {
    setBusy(true);
    setError('');
    try {
      setDraft(await miniAppProduct.inspect.invoke(request));
    } catch {
      setError(t('miniApps.product.importFailed'));
    } finally {
      setBusy(false);
    }
  };
  inspectPath.current = (path) => {
    void inspect({ source_path: path });
  };
  const choose = async (directory = false) => {
    try {
      const paths = await ipcBridge.dialog.showOpen.invoke({
        properties: [directory ? 'openDirectory' : 'openFile'],
        ...(!directory
          ? {
              filters: [
                {
                  name: t('miniApps.title'),
                  extensions: ['nomiapp', 'zip', 'html', 'htm'],
                },
              ],
            }
          : {}),
      });
      if (paths?.[0]) await inspect({ source_path: paths[0] });
    } catch {
      setError(t('miniApps.product.importFailed'));
    }
  };
  const drop = async (event: React.DragEvent) => {
    event.preventDefault();
    if (busy) return;
    const file = event.dataTransfer.files[0];
    if (!file) return;
    const nativePath = (file as File & { path?: string }).path;
    if (nativePath) {
      await inspect({ source_path: nativePath });
      return;
    }
    if (!/\.html?$/i.test(file.name) || file.size > 2_000_000) {
      setError(t('miniApps.product.choosePackage'));
      return;
    }
    await inspect({ filename: file.name, content: await file.text() });
  };
  const cancel = async () => {
    if (busy) return;
    if (draft) {
      try {
        await miniAppProduct.discard.invoke({
          id: draft.id,
          expected_revision: draft.revision,
        });
      } catch {
        setError(t('miniApps.product.operationFailed'));
        return;
      }
    }
    onClose();
  };
  const save = async () => {
    if (!draft || busy) return;
    setBusy(true);
    setError('');
    try {
      const app = await miniAppProduct.save.invoke({
        id: draft.id,
        expected_revision: draft.revision,
      });
      onOpened(app.miniapp.miniapp_id);
    } catch {
      setError(t('miniApps.product.saveFailed'));
      try {
        setDraft(await miniAppProduct.draft.invoke({ id: draft.id }));
      } catch {
        /* Keep the known draft visible. */
      }
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog
      visible={visible}
      style={{ width: preview ? 860 : 540, maxWidth: '95vw' }}
      title={t(
        draft ? 'miniApps.product.importReady' : 'miniApps.product.importTitle',
      )}
      footer={null}
      onCancel={busy ? undefined : () => void cancel()}
      maskClosable={false}
    >
      <div className={styles.form}>
        {error && <Alert type='error' content={error} />}
        {!draft ? (
          <div
            className={styles.drop}
            onDragOver={(e) => e.preventDefault()}
            onDrop={(e) => void drop(e)}
          >
            <Download size={28} />
            <h3>{t('miniApps.product.dropFile')}</h3>
            <p>{t('miniApps.product.importHint')}</p>
            <Button type='primary' loading={busy} onClick={() => void choose()}>
              {t('miniApps.product.chooseFile')}
            </Button>
            <Button
              type='text'
              disabled={busy}
              onClick={() => void choose(true)}
            >
              {t('miniApps.product.chooseFolder')}
            </Button>
          </div>
        ) : (
          <>
            <h3>{draft.name}</h3>
            <p>{draft.description}</p>
            <span className={styles.muted}>
              {t(
                draft.import?.includes_data
                  ? 'miniApps.product.importData'
                  : 'miniApps.product.importNoData',
              )}
            </span>
            <span className={styles.muted}>
              {t(
                draft.import && !draft.import.editable
                  ? 'miniApps.product.runtimeOnly'
                  : 'miniApps.product.editable',
              )}
            </span>
            {draft.import?.requires_service && (
              <Alert
                type='info'
                content={t('miniApps.product.importService')}
              />
            )}
            {preview && draft.html && (
              <div className={styles.importPreview}>
                <MiniAppDraftPreview html={draft.html} title={draft.name} />
              </div>
            )}
            {!draft.html && (
              <p className={styles.muted}>
                {t('miniApps.product.noImportPreview')}
              </p>
            )}
            <div className={styles.toolbar}>
              <Button disabled={busy} onClick={() => void cancel()}>
                {t('miniApps.product.cancel')}
              </Button>
              {draft.html && (
                <Button disabled={busy} onClick={() => setPreview(!preview)}>
                  {t(
                    preview
                      ? 'miniApps.product.hidePreview'
                      : 'miniApps.product.preview',
                  )}
                </Button>
              )}
              <Button type='primary' loading={busy} onClick={() => void save()}>
                {t(
                  draft.import?.includes_data
                    ? 'miniApps.product.restoreOpen'
                    : 'miniApps.product.addOpen',
                )}
              </Button>
            </div>
          </>
        )}
        {busy && (
          <span role='status' className={styles.muted}>
            <Spin size={12} /> {t('miniApps.product.working')}
          </span>
        )}
      </div>
    </Dialog>
  );
}
