import type { AgentPresetDocument, CapabilityCatalogItem, CapabilityId } from '@/common/types/agentPlatform';
import { Alert, Button, Tag } from '@arco-design/web-react';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import { isDesktopShell } from '@/renderer/utils/platform';
import styles from './AgentContextOrder.module.css';

type Props = {
  document: AgentPresetDocument;
  catalog: readonly CapabilityCatalogItem[];
  disabled?: boolean;
  kind?: 'context' | 'middleware';
  onChange: (document: AgentPresetDocument) => void;
  onOpenAuthor?: (destination: string) => void | Promise<void>;
};

export default function AgentContributionOrder({ document, catalog, disabled = false, kind = 'context', onChange, onOpenAuthor }: Props) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const desktopShell = isDesktopShell();
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState(false);
  const pending = useRef(false);
  const currentOpenAuthor = useRef(onOpenAuthor);
  currentOpenAuthor.current = onOpenAuthor;
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const field = kind === 'context' ? 'context_order' : 'middleware_order';
  const labels = kind === 'context' ? 'contextOrder' : 'middlewareOrder';
  const selected = new Set(document.enabled_capabilities.map(value => value.capability.id));
  const contexts = new Map(catalog.filter(item => (kind === 'context' ? item.kind === 'context_contributor' :
    item.kind === 'turn_middleware' || item.middleware_phase !== undefined) &&
    selected.has(item.capability.id)).map(item => [item.capability.id, item]));
  // Retain unavailable explicit choices so a catalog refresh cannot silently change the draft.
  const explicit = document[field] ?? [];
  const ids = [...explicit, ...[...contexts.keys()].filter(id => !explicit.includes(id)).sort()];
  const createBeforeTool = async () => {
    if (!desktopShell || disabled || pending.current) return;
    pending.current = true; setCreating(true); setCreateError(false);
    try {
      const draft = await pluginPlatform.drafts.create.invoke({ template: 'agent.before_tool' });
      if (mounted.current) {
        const destination = `/plugins/create/${encodeURIComponent(draft.summary.draft_id)}`;
        if (currentOpenAuthor.current) await currentOpenAuthor.current(destination);
        else await navigate(destination);
      }
    } catch {
      if (mounted.current) setCreateError(true);
    } finally {
      pending.current = false;
      if (mounted.current) setCreating(false);
    }
  };
  const move = (id: CapabilityId, offset: number) => {
    if (disabled) return;
    const index = ids.indexOf(id), target = index + offset;
    if (index < 0 || target < 0 || target >= ids.length) return;
    const order = [...ids];
    [order[index], order[target]] = [order[target], order[index]];
    onChange({ ...document, [field]: order });
  };
  return <section className={styles.section} aria-label={t(`agentSettings.${labels}.title`)}>
    <h3>{t(`agentSettings.${labels}.title`)}</h3>
    <p>{t(`agentSettings.${labels}.hint`)}</p>
    {kind === 'middleware' && <>
      <p>{t('agentSettings.middlewareOrder.toolAccess')}</p>
      <div className={styles.authoring}>
        <Button size='small' loading={creating} disabled={!desktopShell || disabled || creating} onClick={() => void createBeforeTool()}>
          {t('agentSettings.middlewareOrder.createBeforeTool')}
        </Button>
        <p>{t('agentSettings.middlewareOrder.createHint')}</p>
      </div>
      {createError && <Alert type='warning' content={t('agentSettings.middlewareOrder.createFailed')} />}
    </>}
    {!ids.length && <p>{t(`agentSettings.${labels}.empty`)}</p>}
    <ol className={styles.list}>
      {ids.map((id, index) => {
        const item = contexts.get(id), name = item?.display_name ?? id;
        return <li key={id} className={styles.item}>
          <span className={styles.name}>{name}{!item && <small>{t(`agentSettings.${labels}.missing`)}</small>}</span>
          {kind === 'middleware' && <Tag size='small' className={styles.phase}>
            {t(`agentSettings.middlewareOrder.phase.${item?.middleware_phase === 'before_model' || item?.middleware_phase === 'before_tool'
              ? item.middleware_phase : 'unknown'}`)}
          </Tag>}
          <Button size='small' disabled={disabled || index === 0} aria-label={t(`agentSettings.${labels}.up`, { name })} onClick={() => move(id, -1)}>↑</Button>
          <Button size='small' disabled={disabled || index === ids.length - 1} aria-label={t(`agentSettings.${labels}.down`, { name })} onClick={() => move(id, 1)}>↓</Button>
        </li>;
      })}
    </ol>
    <Button size='small' disabled={disabled || !explicit.length} onClick={() => {
      const next = { ...document };
      delete next[field];
      onChange(next);
    }}>{t(`agentSettings.${labels}.reset`)}</Button>
  </section>;
}
