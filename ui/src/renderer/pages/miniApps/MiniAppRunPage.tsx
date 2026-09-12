import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate, useParams, useSearchParams } from 'react-router-dom';
import { useTranslation } from 'react-i18next';
import {
  Alert,
  Button,
  Input,
  Modal,
  Spin,
  type ModalProps,
} from '@arco-design/web-react';
import { ArrowLeft, More } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { parseMiniAppId } from '@/common/types/ids';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';
import { miniAppProduct } from '@/common/adapter/miniAppProductBridge';
import type {
  MiniAppWorkshop,
  MiniAppSurfaceLaunchDescriptor,
} from '@/common/types/miniAppPlatform';
import MiniAppSurfacePanel from './MiniAppSurfacePanel';
import {
  useMiniAppLibrary,
  updateMiniAppWorkspace,
  emptyItem,
  libraryChanged,
} from './libraryState';
import {
  miniAppRollbackRequest,
  miniAppSetEnabledRequest,
  miniAppTrashRequest,
  miniAppRestoreRequest,
} from './model';
import styles from './MiniAppProduct.module.css';

const Dialog = Modal as unknown as React.ComponentType<
  React.PropsWithChildren<ModalProps>
>;
async function closeSurface(value: MiniAppSurfaceLaunchDescriptor | null) {
  if (value)
    await ipcBridge.miniapps.closeSurface.invoke({
      miniapp_id: value.miniapp_id,
      surface_session_id: value.surface_session_id,
      surface_capability: value.surface_capability,
    });
}
export default function MiniAppRunPage() {
  const { t } = useTranslation(),
    navigate = useNavigate();
  const { id = '' } = useParams();
  const [params] = useSearchParams();
  const { workspace } = useMiniAppLibrary();
  const [app, setApp] = useState<MiniAppWorkshop | null>(null),
    [surface, setSurface] = useState<MiniAppSurfaceLaunchDescriptor | null>(
      null,
    );
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(''),
    [loading, setLoading] = useState(true),
    [more, setMore] = useState(false);
  const [dialog, setDialog] = useState<
      'rename' | 'rollback' | 'trash' | 'backup' | null
    >(null),
    [name, setName] = useState('');
  const activeSurface = useRef<MiniAppSurfaceLaunchDescriptor | null>(null),
    generation = useRef(0);
  const shownName =
    workspace.items[id]?.name ??
    app?.miniapp.display_name ??
    t('miniApps.title');
  const load = useCallback(async () => {
    const run = ++generation.current;
    setLoading(true);
    setError('');
    const old = activeSurface.current;
    activeSurface.current = null;
    setSurface(null);
    try {
      await closeSurface(old);
      const miniapp_id = parseMiniAppId(id);
      const next = await ipcBridge.miniapps.getWorkshop.invoke({ miniapp_id });
      if (generation.current !== run) return;
      setApp(next);
      if (
        next.miniapp.lifecycle === 'enabled' &&
        next.miniapp.releases.active
      ) {
        const descriptor = await ipcBridge.miniapps.openSurface.invoke({
          miniapp_id,
        });
        if (generation.current !== run) {
          await closeSurface(descriptor);
          return;
        }
        activeSurface.current = descriptor;
        setSurface(descriptor);
        void updateMiniAppWorkspace((w) => {
          w.items[id] = {
            ...(w.items[id] ?? emptyItem()),
            last_opened: Date.now(),
          };
        }).catch(() => undefined);
      }
    } catch {
      if (generation.current === run)
        setError(t('miniApps.product.openFailed'));
    } finally {
      if (generation.current === run) setLoading(false);
    }
  }, [id, t]);
  useEffect(() => {
    void load();
    return () => {
      generation.current++;
      const value = activeSurface.current;
      activeSurface.current = null;
      void closeSurface(value).catch(() => undefined);
    };
  }, [load]);
  const action = async (
    operation: (current: MiniAppWorkshop) => Promise<unknown>,
  ) => {
    if (!app || busy) return;
    setBusy(true);
    setError('');
    try {
      const current = await ipcBridge.miniapps.getWorkshop.invoke({
        miniapp_id: app.miniapp.miniapp_id,
      });
      await operation(current);
      setDialog(null);
      setMore(false);
      libraryChanged();
      await load();
    } catch {
      setError(t('miniApps.product.operationFailed'));
    } finally {
      setBusy(false);
    }
  };
  const enabled = (value: boolean) =>
    action(async (current) => {
      const request = miniAppSetEnabledRequest(current, value);
      if (!request) throw new Error('unavailable');
      await ipcBridge.miniapps.setEnabled.invoke(request);
    });
  const restore = () =>
    action(async (current) => {
      const request = miniAppRestoreRequest(current);
      if (!request) throw new Error('unavailable');
      await ipcBridge.miniapps.restore.invoke(request);
    });
  const pin = async () => {
    setBusy(true);
    try {
      await updateMiniAppWorkspace((w) => {
        const item = w.items[id] ?? emptyItem();
        w.items[id] = { ...item, pinned: !item.pinned };
      });
    } catch {
      setError(t('miniApps.product.operationFailed'));
    } finally {
      setBusy(false);
    }
  };
  const share = async (backup = false) => {
    if (!app || busy) return;
    setBusy(true);
    setError('');
    try {
      if (!isTauriRuntime()) {
        setError(t('miniApps.product.desktopShare'));
        return;
      }
      const { save } = await import('@tauri-apps/plugin-dialog');
      const destination_path = await save({
        defaultPath: `${shownName.replace(/[\\/:*?"<>|]/g, '-')}${backup ? '-backup' : ''}.nomiapp`,
        filters: [{ name: t('miniApps.title'), extensions: ['nomiapp'] }],
      });
      if (destination_path) {
        const result = await miniAppProduct.exportFile.invoke({
          miniapp_id: id,
          destination_path,
          backup,
        });
        await load();
        if (!result.resumed) setError(t('miniApps.product.backupResumeFailed'));
      }
    } catch {
      setError(t('miniApps.product.shareFailed'));
    } finally {
      setBusy(false);
    }
  };
  return (
    <main className={styles.page}>
      <header className={styles.header}>
        <Button
          type='text'
          icon={<ArrowLeft />}
          onClick={() => navigate('/mini-apps')}
        >
          {t('miniApps.product.back')}
        </Button>
        <h1>{shownName}</h1>
        {app && app.miniapp.lifecycle !== 'trashed' && (
          <Button
            disabled={busy}
            onClick={() =>
              navigate(
                app.source_state === 'runtime_only'
                  ? '/mini-apps/new'
                  : `/mini-apps/new?app=${id}`,
              )
            }
          >
            {t(
              app.source_state === 'runtime_only'
                ? 'miniApps.product.recreate'
                : 'miniApps.product.edit',
            )}
          </Button>
        )}
        <Button
          type='text'
          icon={<More />}
          aria-expanded={more}
          onClick={() => setMore(!more)}
        >
          {t('miniApps.product.more')}
        </Button>
      </header>
      {params.has('saved') && (
        <div className={styles.notice}>
          {t('miniApps.product.savedNotice')}
          <Button type='text' disabled={busy} onClick={() => void pin()}>
            {t(
              workspace.items[id]?.pinned
                ? 'miniApps.product.unpin'
                : 'miniApps.product.pin',
            )}
          </Button>
        </div>
      )}
      {more && (
        <div className={styles.notice}>
          <Button type='text' disabled={busy} onClick={() => void pin()}>
            {t(
              workspace.items[id]?.pinned
                ? 'miniApps.product.unpin'
                : 'miniApps.product.pin',
            )}
          </Button>
          <Button
            type='text'
            onClick={() => {
              setName(shownName);
              setDialog('rename');
            }}
          >
            {t('miniApps.product.rename')}
          </Button>
          <Button
            type='text'
            disabled={busy || !app?.miniapp.releases.active}
            onClick={() => void share()}
          >
            {t('miniApps.product.share')}
          </Button>
          <Button
            type='text'
            disabled={busy}
            onClick={() => setDialog('backup')}
          >
            {t('miniApps.product.backup')}
          </Button>
          <Button
            type='text'
            disabled={busy || !app?.miniapp.releases.previous}
            onClick={() => setDialog('rollback')}
          >
            {t('miniApps.product.previousVersion')}
          </Button>
          {app?.miniapp.lifecycle === 'enabled' && (
            <Button
              type='text'
              disabled={busy}
              onClick={() => void enabled(false)}
            >
              {t('miniApps.product.disable')}
            </Button>
          )}
          <Button
            type='text'
            disabled={busy || app?.miniapp.lifecycle === 'trashed'}
            onClick={() => setDialog('trash')}
          >
            {t('miniApps.product.moveToTrash')}
          </Button>
        </div>
      )}
      {error && (
        <Alert
          type='error'
          content={error}
          action={
            <Button onClick={() => void load()}>
              {t('miniApps.product.retry')}
            </Button>
          }
        />
      )}
      <section className={styles.runner}>
        {loading ? (
          <div className={styles.empty}>
            <Spin />
          </div>
        ) : surface ? (
          <MiniAppSurfacePanel
            compact
            descriptor={surface}
            displayName={shownName}
            reloading={loading}
            closing={busy}
            onReload={() => void load()}
            onClose={() => navigate('/mini-apps')}
          />
        ) : (
          app && (
            <div className={styles.empty}>
              <h2>{shownName}</h2>
              <p>
                {t(
                  app.miniapp.lifecycle === 'trashed'
                    ? 'miniApps.product.trashedNotice'
                    : app.miniapp.releases.active
                      ? 'miniApps.product.disabledNotice'
                      : 'miniApps.product.unsavedNotice',
                )}
              </p>
              {app.miniapp.lifecycle === 'trashed' ? (
                <Button
                  type='primary'
                  loading={busy}
                  onClick={() => void restore()}
                >
                  {t('miniApps.product.restore')}
                </Button>
              ) : app.miniapp.releases.active ? (
                <Button
                  type='primary'
                  loading={busy}
                  onClick={() => void enabled(true)}
                >
                  {t('miniApps.product.enableOpen')}
                </Button>
              ) : (
                <Button
                  type='primary'
                  onClick={() => navigate(`/mini-apps/new?app=${id}`)}
                >
                  {t('miniApps.product.continue')}
                </Button>
              )}
            </div>
          )
        )}
      </section>
      <Dialog
        visible={Boolean(dialog)}
        title={t(
          dialog === 'rename'
            ? 'miniApps.product.rename'
            : dialog === 'rollback'
              ? 'miniApps.product.previousVersion'
              : dialog === 'backup'
                ? 'miniApps.product.backup'
                : 'miniApps.product.moveToTrash',
        )}
        onCancel={busy ? undefined : () => setDialog(null)}
        confirmLoading={busy}
        okText={t('miniApps.product.confirm')}
        cancelText={t('miniApps.product.cancel')}
        onOk={() => {
          if (dialog === 'rename') {
            if (name.trim())
              void action(() =>
                updateMiniAppWorkspace((w) => {
                  w.items[id] = {
                    ...(w.items[id] ?? emptyItem()),
                    name: name.trim(),
                  };
                }),
              );
          } else if (dialog === 'backup') {
            setDialog(null);
            void share(true);
          } else if (dialog === 'rollback')
            void action(async (current) => {
              const request = miniAppRollbackRequest(current);
              if (!request) throw new Error('unavailable');
              await ipcBridge.miniapps.rollback.invoke(request);
            });
          else
            void action(async (current) => {
              const request = miniAppTrashRequest(current);
              if (!request) throw new Error('unavailable');
              await ipcBridge.miniapps.trash.invoke(request);
            });
        }}
      >
        {dialog === 'rename' ? (
          <Input
            autoFocus
            maxLength={120}
            value={name}
            onChange={setName}
            aria-label={t('miniApps.product.name')}
          />
        ) : (
          <p>
            {t(
              dialog === 'rollback'
                ? 'miniApps.product.rollbackHint'
                : dialog === 'backup'
                  ? 'miniApps.product.backupHint'
                  : 'miniApps.product.trashHint',
            )}
          </p>
        )}
      </Dialog>
    </main>
  );
}
