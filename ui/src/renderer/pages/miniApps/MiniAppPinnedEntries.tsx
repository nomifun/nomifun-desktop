import { useNavigate } from 'react-router-dom';
import { useMiniAppLibrary } from './libraryState';
import styles from './MiniAppProduct.module.css';

export default function MiniAppPinnedEntries({
  collapsed,
}: {
  collapsed: boolean;
}) {
  const { apps, workspace } = useMiniAppLibrary();
  const navigate = useNavigate();
  if (collapsed) return null;
  const pins = apps.filter(
    (app) =>
      workspace.items[app.miniapp_id]?.pinned &&
      app.releases.active &&
      !['trashed', 'deleting'].includes(app.lifecycle),
  );
  if (!pins.length) return null;
  return (
    <div className={styles.pins}>
      {pins.map((app) => (
        <button
          type='button'
          key={app.miniapp_id}
          onClick={() => navigate(`/mini-apps/${app.miniapp_id}`)}
        >
          {workspace.items[app.miniapp_id]?.name ?? app.display_name}
        </button>
      ))}
    </div>
  );
}
