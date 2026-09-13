import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { pluginRuntimeProduct } from '@/common/adapter/pluginRuntimeProductBridge';
import { MINIAPP_LIBRARY_CHANGED } from './runtime/libraryState';
import { pluginProductItems, type PluginProductItem } from './pluginProductModel';
import styles from './runtime/PluginRuntimeProduct.module.css';

export default function PluginPinnedEntries({ collapsed }: { collapsed: boolean }) {
  const [pins, setPins] = useState<PluginProductItem[]>([]);
  const navigate = useNavigate();
  useEffect(() => {
    let active = true;
    let generation = 0;
    const refresh = async () => {
      const request = ++generation;
      try {
        const [library, workspace] = await Promise.all([ipcBridge.plugins.list.invoke(), pluginRuntimeProduct.workspace.invoke()]);
        if (!active || request !== generation) return;
        setPins(pluginProductItems(library).filter((item) => {
          const id = item.runtime?.plugin_id ?? item.mount?.mount_id;
          return id && workspace.items[id]?.pinned && item.status !== 'trashed' && item.runtime?.lifecycle !== 'deleting' && (item.runtime?.releases.active || item.mount?.current);
        }).map((item) => ({ ...item, displayName: workspace.items[item.runtime?.plugin_id ?? item.mount!.mount_id]?.name ?? item.displayName })));
      } catch { /* Retain the last successfully loaded pins during reconnect. */ }
    };
    void refresh();
    window.addEventListener(MINIAPP_LIBRARY_CHANGED, refresh);
    return () => { active = false; window.removeEventListener(MINIAPP_LIBRARY_CHANGED, refresh); };
  }, []);
  if (collapsed || !pins.length) return null;
  return <div className={styles.pins}>{pins.map((item) => (
    <button type='button' key={item.key} onClick={() => navigate(item.runtime ? `/plugins/run/${item.runtime.plugin_id}` : `/plugins?plugin=${item.mount!.mount_id}`)}>{item.displayName}</button>
  ))}</div>;
}
