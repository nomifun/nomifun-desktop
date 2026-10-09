import { useEffect, useState } from 'react';
import SiderNavItem from '@/renderer/components/layout/Sider/SiderNav/SiderNavItem';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { PluginSummary } from '@/common/types/pluginPlatform';
import { subscribePluginLibraryChanges } from './pluginLibraryState';
import styles from './PluginPlatform.module.css';

export default function PluginPinnedEntries({ collapsed }: { collapsed: boolean }) {
  const [plugins, setPlugins] = useState<Array<{ plugin: PluginSummary; name: string }>>([]);

  useEffect(() => {
    let active = true;
    let sequence = 0;
    const refresh = async () => {
      const request = ++sequence;
      try {
        const [library, state] = await Promise.all([
          pluginPlatform.plugins.list.invoke(),
          pluginPlatform.libraryState.get.invoke(),
        ]);
        if (!active || request !== sequence) return;
        const organization = new Map(state.items.map((item) => [item.plugin_id, item]));
        setPlugins(library.plugins.flatMap((plugin) => {
          const item = organization.get(plugin.plugin_id);
          return item?.pinned && plugin.trashed_at_ms === undefined
            ? [{ plugin, name: item.custom_name || plugin.display_name }]
            : [];
        }));
      } catch {
        // Keep the last authoritative pins during reconnect.
      }
    };
    void refresh();
    const off = subscribePluginLibraryChanges(refresh);
    return () => {
      active = false;
      off();
    };
  }, []);

  if (collapsed || !plugins.length) return null;
  return (
    <div className={styles.fileList}>
      {plugins.map(({ plugin, name }) => (
        <SiderNavItem key={plugin.plugin_id} label={name} to={`/plugins/run/${encodeURIComponent(plugin.plugin_id)}`} />
      ))}
    </div>
  );
}
