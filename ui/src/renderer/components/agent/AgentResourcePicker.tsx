import { ipcBridge } from '@/common';
import type { IProvider } from '@/common/config/storage';
import { creativeStudioCanvasApi } from '@/renderer/pages/creativeStudio/services/canvasApi';
import { Button, Select, Spin } from '@arco-design/web-react';
import { LinkOne } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import {
  requiredAgentResourcePickerKinds,
  resolveAgentResourceSelections,
  type AgentResourceSelectionValue,
  type UserAgentResourceKind,
} from '@/renderer/hooks/agent/agentResourceSelection';
import styles from './AgentResourcePicker.module.css';

export type AgentResourceOption = {
  value: string;
  label: string;
  description?: string;
  companionId?: string;
  ownerDomain?: 'companion' | 'customer_service';
  channelIds?: string[];
  knowledgeBaseIds?: string[];
};

export type AgentResourceInventory = {
  options: Partial<Record<UserAgentResourceKind, AgentResourceOption[]>>;
  errors: Partial<Record<UserAgentResourceKind, string>>;
};

export type AgentResourceInventoryLoader = (
  kinds: readonly UserAgentResourceKind[],
  capabilityIds: ReadonlySet<string>,
) => Promise<AgentResourceInventory>;

const emptyInventory = (): AgentResourceInventory => ({ options: {}, errors: {} });

const generationTasks = (capabilityIds: ReadonlySet<string>): string[] => {
  const mapping: Record<string, string> = {
    'creation.text': 'chat',
    'creation.image': 'image_generation',
    'creation.image_edit': 'image_edit',
    'creation.video': 'video_generation',
    'creation.audio': 'speech_synthesis',
  };
  return [...new Set([...capabilityIds].flatMap((id) => mapping[id] ? [mapping[id]] : []))];
};

const providerSupportsTasks = (provider: IProvider, tasks: readonly string[]): boolean =>
  provider.enabled !== false && tasks.every((task) =>
    provider.models.some((model) => model.enabled && model.capabilities.some((capability) => capability.task === task))
  );

export const loadAgentResourceInventory: AgentResourceInventoryLoader = async (kinds, capabilityIds) => {
  const options: AgentResourceInventory['options'] = {};
  const errors: AgentResourceInventory['errors'] = {};
  const wanted = new Set(kinds);
  const jobs: Array<{ kinds: UserAgentResourceKind[]; run: () => Promise<void> }> = [];
  if (wanted.has('companion')) jobs.push({ kinds: ['companion'], run: async () => {
    const rows = await ipcBridge.companion.listCompanions.invoke();
    options.companion = rows.map((row) => ({ value: row.companion_id, label: row.name, description: `#${row.seq}` }));
  }});
  if (wanted.has('channel')) jobs.push({ kinds: ['channel'], run: async () => {
    const rows = await ipcBridge.channel.getPluginStatus.invoke();
    options.channel = rows.filter((row) => row.enabled).map((row) => ({
      value: row.plugin_id, label: row.name, description: row.botUsername ? `${row.type} · @${row.botUsername}` : row.type,
      companionId: row.companionId, ownerDomain: row.owner_domain,
    }));
  }});
  if (wanted.has('robot')) jobs.push({ kinds: ['robot'], run: async () => {
    const rows = await ipcBridge.robot.list.invoke();
    options.robot = rows.map((row) => ({ value: row.robot_id, label: row.name, description: row.board, companionId: row.companion_id ?? undefined }));
  }});
  if (wanted.has('customer')) jobs.push({ kinds: ['customer'], run: async () => {
    const rows = (await ipcBridge.customerService.listAgents.invoke()).filter((row) => row.enabled);
    const channelIds = wanted.has('channel')
      ? await Promise.all(rows.map(async (row) => [row.cs_agent_id, (await ipcBridge.customerService.listBindings.invoke({ cs_agent_id: row.cs_agent_id })).map((binding) => String(binding.channel_plugin_id))] as const))
      : [];
    const channelsByCustomer = new Map(channelIds.map(([id, ids]) => [String(id), ids]));
    options.customer = rows.map((row) => ({
      value: row.cs_agent_id,
      label: row.name,
      knowledgeBaseIds: row.knowledge_base_ids.map(String),
      channelIds: channelsByCustomer.get(String(row.cs_agent_id)),
    }));
  }});
  if (wanted.has('knowledge_base')) jobs.push({ kinds: ['knowledge_base'], run: async () => {
    const needsWrite = [...capabilityIds].some((id) => ['knowledge.write', 'knowledge.autogen', 'knowledge.mount'].includes(id));
    const rows = await ipcBridge.knowledge.listBases.invoke();
    options.knowledge_base = rows.filter((row) => row.root_exists && (!needsWrite || row.tree_access === 'editable')).map((row) => ({
      value: row.knowledge_base_id, label: row.name, description: row.description,
    }));
  }});
  if (wanted.has('mcp_server')) jobs.push({ kinds: ['mcp_server'], run: async () => {
    const rows = await ipcBridge.mcpService.listServers.invoke();
    options.mcp_server = rows.filter((row) => row.enabled).map((row) => ({ value: row.mcp_server_id, label: row.name, description: row.description }));
  }});
  if (wanted.has('canvas')) jobs.push({ kinds: ['canvas'], run: async () => {
    const rows = await creativeStudioCanvasApi.listCanvases();
    options.canvas = rows.map((row) => ({ value: row.canvasId, label: row.title }));
  }});
  if (wanted.has('generation_provider')) jobs.push({ kinds: ['generation_provider'], run: async () => {
    const tasks = generationTasks(capabilityIds);
    const rows = await ipcBridge.mode.listProviders.invoke();
    options.generation_provider = rows.filter((row) => providerSupportsTasks(row, tasks)).map((row) => ({ value: row.id, label: row.name, description: row.platform }));
  }});
  if (wanted.has('miniapp')) jobs.push({ kinds: ['miniapp'], run: async () => {
    const rows = (await ipcBridge.miniapps.library.invoke()).miniapps;
    options.miniapp = rows.filter((row) => row.lifecycle === 'enabled' && row.surface_available).map((row) => ({ value: row.miniapp_id, label: row.display_name, description: row.description }));
  }});

  await Promise.all(jobs.map(async (job) => {
    try { await job.run(); }
    catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      job.kinds.forEach((kind) => { errors[kind] = message; options[kind] = []; });
    }
  }));
  return { options, errors };
};

const routeForKind = (kind: UserAgentResourceKind, value: AgentResourceSelectionValue): string => {
  if (kind === 'channel') return value.customer ? `/customer-service/${value.customer}` : '/nomi';
  if (kind === 'companion' || kind === 'robot') return '/nomi';
  if (kind === 'customer') return '/customer-service';
  if (kind === 'knowledge_base') return value.customer ? `/customer-service/${value.customer}` : '/knowledge';
  if (kind === 'mcp_server') return '/mcp';
  if (kind === 'canvas') return '/creative-studio/canvases';
  if (kind === 'generation_provider') return '/models';
  if (kind === 'miniapp') return '/mini-apps';
  return '/guid';
};

export const optionsForAgentResourceField = (
  kind: UserAgentResourceKind,
  inventory: AgentResourceInventory,
  value: AgentResourceSelectionValue,
  required: ReadonlySet<string>,
): AgentResourceOption[] => {
  let options = inventory.options[kind] ?? [];
  const companionId = value.companion;
  const customer = (inventory.options.customer ?? []).find((item) => item.value === value.customer);
  if (kind === 'channel') {
    if (required.has('customer')) {
      if (!customer) return [];
      const allowed = new Set(customer.channelIds ?? []);
      options = options.filter((item) => item.ownerDomain === 'customer_service' && allowed.has(item.value));
    } else if (required.has('companion') || required.has('companion_memory')) {
      if (!companionId) return [];
      options = options.filter((item) => item.ownerDomain === 'companion' && item.companionId === companionId);
    }
  }
  if (kind === 'robot' && (required.has('companion') || required.has('companion_memory'))) {
    if (!companionId) return [];
    options = options.filter((item) => item.companionId === companionId);
  }
  if (kind === 'knowledge_base' && required.has('customer')) {
    if (!customer) return [];
    const allowed = new Set(customer.knowledgeBaseIds ?? []);
    options = options.filter((item) => allowed.has(item.value));
  }
  return options;
};

type Props = {
  requiredKinds: ReadonlySet<string> | readonly string[];
  capabilityIds: ReadonlySet<string> | readonly string[];
  value: AgentResourceSelectionValue;
  onChange: (value: AgentResourceSelectionValue) => void;
  disabled?: boolean;
  loadInventory?: AgentResourceInventoryLoader;
  onNavigateToResource?: (route: string) => void;
};

const AgentResourcePicker: React.FC<Props> = ({ requiredKinds, capabilityIds, value, onChange, disabled = false, loadInventory = loadAgentResourceInventory, onNavigateToResource }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const requiredKey = [...requiredKinds].sort().join('\u0000');
  const capabilityKey = [...capabilityIds].sort().join('\u0000');
  const required = useMemo(() => new Set(requiredKey ? requiredKey.split('\u0000') : []), [requiredKey]);
  const capabilities = useMemo(() => new Set(capabilityKey ? capabilityKey.split('\u0000') : []), [capabilityKey]);
  const fields = useMemo(() => requiredAgentResourcePickerKinds(required), [required]);
  const [inventory, setInventory] = useState<AgentResourceInventory>(emptyInventory);
  const [loading, setLoading] = useState(fields.length > 0);
  const resolution = useMemo(() => resolveAgentResourceSelections(required, value), [required, value]);
  const missingChoiceCount = fields.filter((kind) => !value[kind]).length;

  useEffect(() => {
    let cancelled = false;
    if (!fields.length) { setInventory(emptyInventory()); setLoading(false); return undefined; }
    setLoading(true);
    void loadInventory(fields, capabilities)
      .then((next) => { if (!cancelled) setInventory(next); })
      .catch((error) => {
        if (cancelled) return;
        const message = error instanceof Error ? error.message : String(error);
        setInventory({ options: {}, errors: Object.fromEntries(fields.map((kind) => [kind, message])) });
      })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [capabilities, fields, loadInventory]);

  useEffect(() => {
    if (loading) return;
    const next = { ...value };
    let changed = false;
    for (const kind of fields) {
      if (next[kind] && !optionsForAgentResourceField(kind, inventory, value, required).some((option) => option.value === next[kind])) {
        delete next[kind]; changed = true;
      }
    }
    if (changed) onChange(next);
  }, [fields, inventory, loading, onChange, required, value]);

  if (!fields.length) return null;
  const kindName = (kind: UserAgentResourceKind) => t(`agentSettings.resources.kinds.${kind === 'mcp_server' ? 'mcpConnection' : kind.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase())}`);
  return <section className={styles.picker} aria-label={t('agentSettings.resources.pickerTitle')}>
    <div className={styles.header}><div className={styles.headerCopy}><strong>{t('agentSettings.resources.pickerTitle')}</strong><span>{t('agentSettings.resources.pickerHint')}</span></div>
      <span className={`${styles.status} ${!resolution.missingKinds.length ? styles.statusReady : ''}`}>{t(resolution.missingKinds.length ? 'agentSettings.resources.missingCount' : 'agentSettings.resources.ready', { count: missingChoiceCount })}</span>
    </div>
    <div className={styles.fields}>{fields.map((kind) => {
      const options = optionsForAgentResourceField(kind, inventory, value, required);
      const error = inventory.errors[kind];
      const dependsOn = kind === 'channel' || kind === 'robot' ? (required.has('customer') ? 'customer' : required.has('companion') || required.has('companion_memory') ? 'companion' : undefined) : kind === 'knowledge_base' && required.has('customer') ? 'customer' : undefined;
      const waitingDependency = Boolean(dependsOn && !value[dependsOn as UserAgentResourceKind]);
      return <label className={styles.field} key={kind}><span className={styles.fieldLabel}>{kindName(kind)}</span><span className={styles.fieldControl}>
        <Select value={value[kind]} disabled={disabled || loading || waitingDependency || options.length === 0} placeholder={t(waitingDependency ? 'agentSettings.resources.selectDependency' : 'agentSettings.resources.selectPlaceholder', { resource: dependsOn ? kindName(dependsOn as UserAgentResourceKind) : kindName(kind) })} aria-label={t('agentSettings.resources.selectAria', { resource: kindName(kind) })} onChange={(resourceId: string) => onChange({ ...value, [kind]: resourceId })}>
          {options.map((option) => <Select.Option key={option.value} value={option.value}><span className={styles.option}><strong>{option.label}</strong>{option.description && <small>{option.description}</small>}</span></Select.Option>)}
        </Select>
        {!loading && !waitingDependency && options.length === 0 && <Button className={styles.emptyButton} size='mini' type='text' icon={<LinkOne theme='outline' size={13} />} onClick={(event) => { event.preventDefault(); const route = routeForKind(kind, value); if (onNavigateToResource) onNavigateToResource(route); else void navigate(route); }}>{t('agentSettings.resources.configure')}</Button>}
      </span><span className={`${styles.fieldHint} ${error ? styles.fieldError : ''}`}>{loading ? <Spin size={10} /> : error ? t('agentSettings.resources.loadFailed') : waitingDependency ? t('agentSettings.resources.dependencyHint', { resource: kindName(dependsOn as UserAgentResourceKind) }) : options.length === 0 ? t('agentSettings.resources.emptyOptions') : t('agentSettings.resources.selectionHint')}</span></label>;
    })}</div>
  </section>;
};

export default AgentResourcePicker;
