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
import { parsePluginRuntimeId } from '@/common/types/ids';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';
import { pluginRuntimeProduct } from '@/common/adapter/pluginRuntimeProductBridge';
import type {
  PluginRuntimeWorkshop,
  PluginRuntimeSurfaceLaunchDescriptor,
} from '@/common/types/pluginRuntimePlatform';
import PluginRuntimeSurfacePanel from './PluginRuntimeSurfacePanel';
import {
  usePluginRuntimeLibrary,
  updatePluginRuntimeWorkspace,
  emptyItem,
  libraryChanged,
} from './libraryState';
import {
  pluginRuntimeRollbackRequest,
  pluginRuntimeSetEnabledRequest,
  pluginRuntimeTrashRequest,
  pluginRuntimeRestoreRequest,
  pluginRuntimeDeleteRequest,
  pluginRuntimeRetryServiceRequest,
} from './model';
import styles from './PluginRuntimeProduct.module.css';

const Dialog = Modal as unknown as React.ComponentType<
  React.PropsWithChildren<ModalProps>
>;
async function closeSurface(value: PluginRuntimeSurfaceLaunchDescriptor | null) {
  if (value)
    await ipcBridge.pluginRuntimes.closeSurface.invoke({
      plugin_id: value.plugin_id,
      surface_session_id: value.surface_session_id,
      surface_capability: value.surface_capability,
    });
}
export default function PluginRuntimeRunPage() {
  const { t } = useTranslation(),
    navigate = useNavigate();
  const { id = '' } = useParams();
  const [params] = useSearchParams();
  const { workspace } = usePluginRuntimeLibrary();
  const [app, setApp] = useState<PluginRuntimeWorkshop | null>(null),
    [surface, setSurface] = useState<PluginRuntimeSurfaceLaunchDescriptor | null>(
      null,
    );
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(''),
    [loading, setLoading] = useState(true),
    [more, setMore] = useState(false);
  const [dialog, setDialog] = useState<
      'rename' | 'rollback' | 'trash' | 'backup' | 'delete' | null
    >(null),
    [name, setName] = useState('');
  const activeSurface = useRef<PluginRuntimeSurfaceLaunchDescriptor | null>(null),
    generation = useRef(0);
  const shownName =
    workspace.items[id]?.name ??
    app?.plugin.display_name ??
    t('pluginRuntime.title');
  const load = useCallback(async () => {
    const run = ++generation.current;
    setLoading(true);
    setError('');
    const old = activeSurface.current;
    activeSurface.current = null;
    setSurface(null);
    try {
      await closeSurface(old);
      const plugin_id = parsePluginRuntimeId(id);
      const next = await ipcBridge.pluginRuntimes.getWorkshop.invoke({ plugin_id });
      if (generation.current !== run) return;
      setApp(next);
      if (
        next.plugin.lifecycle === 'enabled' &&
        next.plugin.releases.active && next.plugin.surface_available
      ) {
        const descriptor = await ipcBridge.pluginRuntimes.openSurface.invoke({
          plugin_id,
        });
        if (generation.current !== run) {
          await closeSurface(descriptor);
          return;
        }
        activeSurface.current = descriptor;
        setSurface(descriptor);
        void updatePluginRuntimeWorkspace((w) => {
          w.items[id] = {
            ...(w.items[id] ?? emptyItem()),
            last_opened: Date.now(),
          };
        }).catch(() => undefined);
      }
    } catch {
      if (generation.current === run)
        setError(t('pluginRuntime.product.openFailed'));
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
    operation: (current: PluginRuntimeWorkshop) => Promise<unknown>,
  ) => {
    if (!app || busy) return;
    setBusy(true);
    setError('');
    try {
      const current = await ipcBridge.pluginRuntimes.getWorkshop.invoke({
        plugin_id: app.plugin.plugin_id,
      });
      await operation(current);
      setDialog(null);
      setMore(false);
      libraryChanged();
      await load();
    } catch {
      setError(t('pluginRuntime.product.operationFailed'));
    } finally {
      setBusy(false);
    }
  };
  const enabled = (value: boolean) =>
    action(async (current) => {
      const request = pluginRuntimeSetEnabledRequest(current, value);
      if (!request) throw new Error('unavailable');
      await ipcBridge.pluginRuntimes.setEnabled.invoke(request);
    });
  const restore = () =>
    action(async (current) => {
      const request = pluginRuntimeRestoreRequest(current);
      if (!request) throw new Error('unavailable');
      await ipcBridge.pluginRuntimes.restore.invoke(request);
    });
  const pin = async () => {
    setBusy(true);
    try {
      await updatePluginRuntimeWorkspace((w) => {
        const item = w.items[id] ?? emptyItem();
        w.items[id] = { ...item, pinned: !item.pinned };
      });
    } catch {
      setError(t('pluginRuntime.product.operationFailed'));
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
        setError(t('pluginRuntime.product.desktopShare'));
        return;
      }
      const { save } = await import('@tauri-apps/plugin-dialog');
      const destination_path = await save({
        defaultPath: `${shownName.replace(/[\\/:*?"<>|]/g, '-')}${backup ? '-backup' : ''}.nomiplugin`,
        filters: [{ name: t('pluginRuntime.title'), extensions: ['nomiplugin'] }],
      });
      if (destination_path) {
        const result = await pluginRuntimeProduct.exportFile.invoke({
          plugin_id: id,
          destination_path,
          backup,
        });
        await load();
        if (!result.resumed) setError(t('pluginRuntime.product.backupResumeFailed'));
      }
    } catch {
      setError(t('pluginRuntime.product.shareFailed'));
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
          onClick={() => navigate('/plugins')}
        >
          {t('pluginRuntime.product.back')}
        </Button>
        <h1>{shownName}</h1>
        {app && app.plugin.lifecycle !== 'trashed' && (
          <Button
            disabled={busy}
            onClick={() =>
              navigate(
                app.source_state === 'runtime_only'
                  ? '/plugins/new'
                  : `/plugins/new?app=${id}`,
              )
            }
          >
            {t(
              app.source_state === 'runtime_only'
                ? 'pluginRuntime.product.recreate'
                : 'pluginRuntime.product.edit',
            )}
          </Button>
        )}
        <Button
          type='text'
          icon={<More />}
          aria-expanded={more}
          onClick={() => setMore(!more)}
        >
          {t('pluginRuntime.product.more')}
        </Button>
      </header>
      {params.has('saved') && (
        <div className={styles.notice}>
          {t('pluginRuntime.product.savedNotice')}
          <Button type='text' disabled={busy} onClick={() => void pin()}>
            {t(
              workspace.items[id]?.pinned
                ? 'pluginRuntime.product.unpin'
                : 'pluginRuntime.product.pin',
            )}
          </Button>
        </div>
      )}
      {more && (
        <div className={styles.notice}>
          <Button type='text' disabled={busy} onClick={() => void pin()}>
            {t(
              workspace.items[id]?.pinned
                ? 'pluginRuntime.product.unpin'
                : 'pluginRuntime.product.pin',
            )}
          </Button>
          <Button
            type='text'
            onClick={() => {
              setName(shownName);
              setDialog('rename');
            }}
          >
            {t('pluginRuntime.product.rename')}
          </Button>
          <Button
            type='text'
            disabled={busy || !app?.plugin.releases.active}
            onClick={() => void share()}
          >
            {t('pluginRuntime.product.share')}
          </Button>
          <Button
            type='text'
            disabled={busy}
            onClick={() => setDialog('backup')}
          >
            {t('pluginRuntime.product.backup')}
          </Button>
          <Button
            type='text'
            disabled={busy || !app?.plugin.releases.previous}
            onClick={() => setDialog('rollback')}
          >
            {t('pluginRuntime.product.previousVersion')}
          </Button>
          {app?.plugin.lifecycle === 'enabled' && (
            <Button
              type='text'
              disabled={busy}
              onClick={() => void enabled(false)}
            >
              {t('pluginRuntime.product.disable')}
            </Button>
          )}
          <Button
            type='text'
            disabled={busy || app?.plugin.lifecycle === 'trashed'}
            onClick={() => setDialog('trash')}
          >
            {t('pluginRuntime.product.moveToTrash')}
          </Button>
        </div>
      )}
      {error && (
        <Alert
          type='error'
          content={error}
          action={
            <Button onClick={() => void load()}>
              {t('pluginRuntime.product.retry')}
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
          <PluginRuntimeSurfacePanel
            compact
            descriptor={surface}
            displayName={shownName}
            reloading={loading}
            closing={busy}
            onReload={() => void load()}
            onClose={() => navigate('/plugins')}
          />
        ) : (
          app && (
            <div className={styles.empty}>
              <h2>{shownName}</h2>
              <p>
                {t(
                  app.plugin.lifecycle === 'trashed'
                    ? 'pluginRuntime.product.trashedNotice'
                    : app.plugin.service_health.state === 'failed'
                      ? 'pluginRuntime.product.backgroundFailed'
                    : app.plugin.lifecycle === 'enabled'
                      ? 'pluginRuntime.product.backgroundEnabled'
                    : app.plugin.releases.active
                      ? 'pluginRuntime.product.disabledNotice'
                      : 'pluginRuntime.product.unsavedNotice',
                )}
              </p>
              {app.plugin.lifecycle === 'trashed' ? (
                <Button
                  type='primary'
                  loading={busy}
                  onClick={() => void restore()}
                >
                  {t('pluginRuntime.product.restore')}
                </Button>
              ) : app.plugin.lifecycle === 'enabled' ? (
                <>
                  {app.capabilities.map((capability) => <p key={capability.capability.id}>{capability.display_name}</p>)}
                  {app.plugin.service_health.state === 'failed' && <Button type='primary' loading={busy} onClick={() => void action(async (current) => {
                    const request = pluginRuntimeRetryServiceRequest(current);
                    if (!request) throw new Error('unavailable');
                    await ipcBridge.pluginRuntimes.retryService.invoke(request);
                  })}>{t('pluginRuntime.product.retry')}</Button>}
                  <Button loading={busy} onClick={() => void enabled(false)}>{t('pluginRuntime.product.disable')}</Button>
                </>
              ) : app.plugin.releases.active ? (
                <Button
                  type='primary'
                  loading={busy}
                  onClick={() => void enabled(true)}
                >
                  {t('pluginRuntime.product.enableOpen')}
                </Button>
              ) : (
                <Button
                  type='primary'
                  onClick={() => navigate(`/plugins/new?app=${id}`)}
                >
                  {t('pluginRuntime.product.continue')}
                </Button>
              )}
            </div>
          )
        )}
      </section>
      {app?.plugin.lifecycle === 'trashed' && <Button status='danger' disabled={busy} onClick={() => setDialog('delete')}>{t('pluginRuntime.product.permanentDelete')}</Button>}
      <Dialog
        visible={Boolean(dialog)}
        title={t(
          dialog === 'rename'
            ? 'pluginRuntime.product.rename'
            : dialog === 'rollback'
              ? 'pluginRuntime.product.previousVersion'
              : dialog === 'delete'
                ? 'pluginRuntime.product.permanentDelete'
              : dialog === 'backup'
                ? 'pluginRuntime.product.backup'
                : 'pluginRuntime.product.moveToTrash',
        )}
        onCancel={busy ? undefined : () => setDialog(null)}
        confirmLoading={busy}
        okText={t('pluginRuntime.product.confirm')}
        cancelText={t('pluginRuntime.product.cancel')}
        onOk={() => {
          if (dialog === 'rename') {
            if (name.trim())
              void action(() =>
                updatePluginRuntimeWorkspace((w) => {
                  w.items[id] = {
                    ...(w.items[id] ?? emptyItem()),
                    name: name.trim(),
                  };
                }),
              );
          } else if (dialog === 'delete') {
            void action(async (current) => {
              const request = pluginRuntimeDeleteRequest(current);
              if (!request) throw new Error('unavailable');
              await ipcBridge.pluginRuntimes.delete.invoke(request);
              navigate('/plugins');
            });
          } else if (dialog === 'backup') {
            setDialog(null);
            void share(true);
          } else if (dialog === 'rollback')
            void action(async (current) => {
              const request = pluginRuntimeRollbackRequest(current);
              if (!request) throw new Error('unavailable');
              await ipcBridge.pluginRuntimes.rollback.invoke(request);
            });
          else
            void action(async (current) => {
              const request = pluginRuntimeTrashRequest(current);
              if (!request) throw new Error('unavailable');
              await ipcBridge.pluginRuntimes.trash.invoke(request);
            });
        }}
      >
        {dialog === 'rename' ? (
          <Input
            autoFocus
            maxLength={120}
            value={name}
            onChange={setName}
            aria-label={t('pluginRuntime.product.name')}
          />
        ) : (
          <p>
            {t(
              dialog === 'rollback'
                ? 'pluginRuntime.product.rollbackHint'
                : dialog === 'delete'
                  ? 'pluginRuntime.product.permanentDeleteHint'
                : dialog === 'backup'
                  ? 'pluginRuntime.product.backupHint'
                  : 'pluginRuntime.product.trashHint',
            )}
          </p>
        )}
      </Dialog>
    </main>
  );
}
