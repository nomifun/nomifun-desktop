import { useId, useState } from 'react';
import { Copy, Right } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import type { ToolFileSummary } from '@/common/chat/fileSummary';
import { projectFileActivity } from '../fileActivityModel';

export type FileActivityInput = { key: string; summary: ToolFileSummary };

/** Shorten only a proven workspace prefix, preserving the rest of the path. */
const fileLabel = (path: string, roots: readonly string[]): string => {
  const comparable = path.replace(/\\/g, '/');
  const root = roots.map(root => root.replace(/\\/g, '/').replace(/\/+$/, ''))
    .filter(root => root && comparable.startsWith(`${root}/`)).sort((a, b) => b.length - a.length)[0];
  return root ? path.slice(root.length + 1) : path;
};

/** Paths and verified change counts only; this component never reads tool bodies. */
export function FileActivityList({ items, workspaceRoots, confirmed }: {
  items: readonly FileActivityInput[]; workspaceRoots: readonly string[]; confirmed: boolean;
}) {
  const { t } = useTranslation();
  const view = projectFileActivity([...items]);
  return <div className='turn-file-activity__list' role='list'>
    {view.entries.map(entry => <div key={entry.id} role='listitem' className='turn-file-activity__entry'>
      <span className='turn-file-activity__verb'>{t(confirmed ? `messages.fileActivity.action.${entry.action}` : 'messages.fileActivity.target')}</span>{' '}
      <span className='turn-file-activity__path' title={entry.path}>{fileLabel(entry.path, workspaceRoots)}
        {entry.destination && <> → <span title={entry.destination}>{fileLabel(entry.destination, workspaceRoots)}</span></>}
      </span>
      {confirmed && entry.addedLines !== undefined && entry.removedLines !== undefined &&
        <span className='turn-file-activity__changes'>+{entry.addedLines} −{entry.removedLines}</span>}
    </div>)}
  </div>;
}

export default function FileActivitySummary({ items, workspaceRoots }: { items: readonly FileActivityInput[]; workspaceRoots: readonly string[] }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const id = useId();
  const view = projectFileActivity([...items]);
  const summary = (['read', 'edit', 'delete', 'rename', 'instructions'] as const)
    .filter(action => view.counts[action] > 0)
    .map(action => t(`messages.fileActivity.count.${action}`, { count: view.counts[action] }))
    .join(t('messages.fileActivity.separator'));
  if (!view.entries.length) return null;
  return <div className='turn-file-activity'>
    <button type='button' className='turn-process-trace__row turn-file-activity__toggle' aria-expanded={expanded}
      aria-controls={id} onClick={() => setExpanded(value => !value)}>
      <span className='turn-process-trace__row-icon'><Copy theme='outline' size='14' /></span>
      <span className='turn-process-trace__text'>{summary}</span>
      <Right theme='outline' size='12' className={`turn-process-trace-tool__arrow${expanded ? ' turn-process-trace-tool__arrow--open' : ''}`} />
    </button>
    {expanded && <div id={id}><FileActivityList items={items} workspaceRoots={workspaceRoots} confirmed /></div>}
  </div>;
}
