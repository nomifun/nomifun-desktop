import { ipcBridge } from '@/common';
import type { ComputerPermissionStatus, IApiRobotPermissions, IApiRobotPhase } from '@/common/adapter/ipcBridge';
import { creativeStudioCanvasApi } from '@/renderer/pages/creativeStudio/services/canvasApi';
import { Button, Select, Spin } from '@arco-design/web-react';
import { LinkOne } from '@icon-park/react';
import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import CompanionAvatar from '@/renderer/pages/companion/CompanionAvatar';
import { customFigureMetaOf } from '@/renderer/pages/companion/characters/customMeta';
import {
  requiredAgentResourcePickerKinds,
  resolveAgentResourceSelections,
  selectedMcpResourceIds,
  allowsMultipleMcpServers,
  type AgentResourceSelectionValue,
  type UserAgentResourceKind,
} from '@/renderer/hooks/agent/agentResourceSelection';
import styles from './AgentResourcePicker.module.css';

export type AgentResourceOption = {
  value: string;
  label: string;
  description?: string;
  avatar?: Pick<React.ComponentProps<typeof CompanionAvatar>, 'character' | 'companionId' | 'customFigure'>;
  companionId?: string;
  ownerDomain?: 'companion' | 'customer_service';
  channelIds?: string[];
  knowledgeBaseIds?: string[];
  selectable?: boolean;
  robotPhase?: IApiRobotPhase;
  robotRequiredPermissions?: Array<keyof IApiRobotPermissions>;
  robotDisabledPermissions?: Array<keyof IApiRobotPermissions>;
  robotUnsupportedPermissions?: Array<keyof IApiRobotPermissions>;
};

export type AgentResourceInventory = {
  options: Partial<Record<UserAgentResourceKind, AgentResourceOption[]>>;
  errors: Partial<Record<UserAgentResourceKind, string>>;
};

export type AgentResourceInventoryLoader = (
  kinds: readonly UserAgentResourceKind[],
  capabilityIds: ReadonlySet<string>,
  actionIds?: ReadonlySet<string>,
) => Promise<AgentResourceInventory>;

const emptyInventory = (): AgentResourceInventory => ({ options: {}, errors: {} });

const ROBOT_PERMISSION_BY_ACTION: Readonly<Partial<Record<string, keyof IApiRobotPermissions>>> = {
  'robot/vision': 'vision',
  'robot/display': 'display',
  'robot/motion': 'motion',
  'robot/device': 'device_tools',
};

export const loadAgentResourceInventory: AgentResourceInventoryLoader = async (
  kinds,
  _capabilityIds,
  actionIds = new Set<string>(),
) => {
  const options: AgentResourceInventory['options'] = {};
  const errors: AgentResourceInventory['errors'] = {};
  const wanted = new Set(kinds);
  const jobs: Array<{ kinds: UserAgentResourceKind[]; run: () => Promise<void> }> = [];
  if (wanted.has('companion')) jobs.push({ kinds: ['companion'], run: async () => {
    const rows = await ipcBridge.companion.listCompanions.invoke();
    options.companion = rows.map((row) => ({
      value: row.companion_id, label: row.name, description: `#${row.seq}`,
      avatar: { character: row.character, companionId: row.companion_id, customFigure: customFigureMetaOf(row) },
    }));
  }});
  if (wanted.has('channel')) jobs.push({ kinds: ['channel'], run: async () => {
    const rows = await ipcBridge.channel.getPluginStatus.invoke();
    options.channel = rows.filter((row) => row.enabled).map((row) => ({
      value: row.plugin_id, label: row.name, description: row.botUsername ? `${row.type} · @${row.botUsername}` : row.type,
      companionId: row.companionId, ownerDomain: row.owner_domain,
    }));
  }});
  if (wanted.has('robot')) jobs.push({ kinds: ['robot'], run: async () => {
    const [rows, statuses] = await Promise.all([
      ipcBridge.robot.list.invoke(),
      ipcBridge.robot.statuses.invoke(),
    ]);
    const phaseByRobot = new Map(statuses.map((status) => [status.robot_id, status.phase]));
    const requiredPermissions = [...actionIds]
      .map((action) => ROBOT_PERMISSION_BY_ACTION[action])
      .filter((permission): permission is keyof IApiRobotPermissions => Boolean(permission));
    options.robot = rows.map((row) => {
      const phase = phaseByRobot.get(row.robot_id) ?? 'offline';
      const unsupportedPermissions = requiredPermissions.filter(
        (permission) => !row.supported_permissions.includes(permission)
      );
      const disabledPermissions = requiredPermissions.filter(
        (permission) => row.supported_permissions.includes(permission) && !row.permissions[permission]
      );
      return {
        value: row.robot_id,
        label: row.name,
        description: row.board,
        companionId: row.companion_id ?? undefined,
        selectable: phase !== 'offline'
          && unsupportedPermissions.length === 0
          && disabledPermissions.length === 0,
        robotPhase: phase,
        robotRequiredPermissions: requiredPermissions,
        robotDisabledPermissions: disabledPermissions,
        robotUnsupportedPermissions: unsupportedPermissions,
      };
    });
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
    const needsWrite = [...actionIds].some((id) =>
      ['knowledge/write', 'knowledge/autogen'].includes(id)
    );
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
  if (wanted.has('plugin')) jobs.push({ kinds: ['plugin'], run: async () => {
    const rows = (await ipcBridge.pluginRuntimes.library.invoke()).plugins;
    options.plugin = rows.filter((row) => row.lifecycle === 'enabled' && row.surface_available).map((row) => ({ value: row.plugin_id, label: row.display_name, description: row.description }));
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
  if (kind === 'plugin') return '/plugins';
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
  optionalKinds?: readonly UserAgentResourceKind[];
  companionBindings?: boolean;
  capabilityIds: ReadonlySet<string> | readonly string[];
  actionIds?: ReadonlySet<string> | readonly string[];
  value: AgentResourceSelectionValue;
  onChange: (value: AgentResourceSelectionValue) => void;
  onAvailabilityChange?: (ready: boolean) => void;
  disabled?: boolean;
  loadInventory?: AgentResourceInventoryLoader;
  onNavigateToResource?: (route: string) => void;
};

const AgentResourcePicker: React.FC<Props> = ({ requiredKinds, optionalKinds = [], companionBindings = false, capabilityIds, actionIds = [], value, onChange, onAvailabilityChange, disabled = false, loadInventory = loadAgentResourceInventory, onNavigateToResource }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const requiredKey = [...requiredKinds].sort().join('\u0000');
  const capabilityKey = [...capabilityIds].sort().join('\u0000');
  const actionKey = [...actionIds].sort().join('\u0000');
  const required = useMemo(() => new Set(requiredKey ? requiredKey.split('\u0000') : []), [requiredKey]);
  const capabilities = useMemo(() => new Set(capabilityKey ? capabilityKey.split('\u0000') : []), [capabilityKey]);
  const actions = useMemo(() => new Set(actionKey ? actionKey.split('\u0000') : []), [actionKey]);
  const computerRequired = required.has('computer');
  const [computerPermissions, setComputerPermissions] = useState<ComputerPermissionStatus | null>(null);
  const [computerLoading, setComputerLoading] = useState(computerRequired);
  const [computerLoadFailed, setComputerLoadFailed] = useState(false);
  const multipleMcp = companionBindings || allowsMultipleMcpServers(capabilities);
  const [rosterRevision, setRosterRevision] = useState(0);
  useEffect(() => {
    if (!computerRequired) {
      setComputerPermissions(null);
      setComputerLoading(false);
      setComputerLoadFailed(false);
      return;
    }
    let cancelled = false;
    const refresh = () => {
      setComputerLoading(true);
      void ipcBridge.computerPermissions.get.invoke().then((status) => {
        if (!cancelled) { setComputerPermissions(status); setComputerLoadFailed(false); }
      }).catch(() => {
        if (!cancelled) { setComputerPermissions(null); setComputerLoadFailed(true); }
      }).finally(() => { if (!cancelled) setComputerLoading(false); });
    };
    refresh();
    window.addEventListener('focus', refresh);
    return () => { cancelled = true; window.removeEventListener('focus', refresh); };
  }, [computerRequired]);
  useEffect(() => {
    if (!required.has('companion') && !required.has('companion_memory')) return;
    const refresh = () => setRosterRevision((previous) => previous + 1);
    const unsubscribe = [ipcBridge.companion.onCompanionCreated.on(refresh), ipcBridge.companion.onCompanionDeleted.on(refresh)];
    return () => unsubscribe.forEach((stop) => stop());
  }, [required]);
  useEffect(() => {
    if (!required.has('robot') && !optionalKinds.includes('robot')) return;
    const refresh = () => setRosterRevision((previous) => previous + 1);
    return ipcBridge.robot.onStatus.on(refresh);
  }, [optionalKinds, required]);
  const optionalKey = optionalKinds.join('\u0000');
  const fields = useMemo(() => requiredAgentResourcePickerKinds([...required, ...optionalKey.split('\u0000')]), [required, optionalKey]);
  const [inventory, setInventory] = useState<AgentResourceInventory>(emptyInventory);
  const [pending, setPending] = useState<ReadonlySet<UserAgentResourceKind>>(() => new Set(fields));
  const loading = pending.size > 0;
  const resolution = useMemo(() => resolveAgentResourceSelections(required, value), [required, value]);
  const missingChoiceCount = requiredAgentResourcePickerKinds(resolution.missingKinds).length;
  const fieldOptions = (kind: UserAgentResourceKind, selection = value) => {
    if (companionBindings && (kind === 'channel' || kind === 'robot')) {
      if (!selection.companion) return [];
      return (inventory.options[kind] ?? []).filter((item) =>
        (kind !== 'channel' || item.ownerDomain === 'companion') &&
        (!item.companionId || item.companionId === selection.companion));
    }
    return optionsForAgentResourceField(kind, inventory, selection, required);
  };

  useEffect(() => {
    let cancelled = false;
    setPending(new Set(fields));
    setInventory(emptyInventory());
    // A slow MCP/device request must not disable the independently loaded roster.
    for (const kind of fields) {
      const kinds = kind === 'customer' && fields.includes('channel') ? [kind, 'channel'] as const : [kind];
      void loadInventory(kinds, capabilities, actions).then((next) => {
        if (!cancelled) setInventory((previous) => ({
          options: { ...previous.options, [kind]: next.options[kind] ?? [] },
          errors: { ...previous.errors, [kind]: next.errors[kind] },
        }));
      }).catch((error) => {
        if (!cancelled) setInventory((previous) => ({ ...previous,
          errors: { ...previous.errors, [kind]: String(error) },
        }));
      }).finally(() => {
        if (!cancelled) setPending((previous) => new Set([...previous].filter((item) => item !== kind)));
      });
    }
    return () => { cancelled = true; };
  }, [actions, capabilities, fields, loadInventory, rosterRevision]);

  useEffect(() => {
    if (loading) return;
    const next = { ...value };
    let changed = false;
    for (const kind of fields) {
      if (inventory.errors[kind]) continue;
      if (kind === 'mcp_server' && multipleMcp) {
        const selected = selectedMcpResourceIds(next);
        const available = new Set(optionsForAgentResourceField(kind, inventory, value, required).map((option) => option.value));
        const retained = selected.filter((id) => available.has(id)).slice(0, 16);
        if (retained.length !== selected.length) {
          next.mcp_servers = retained; delete next.mcp_server; changed = true;
        }
        continue;
      }
      if (kind === 'mcp_server' && next.mcp_servers !== undefined) {
        const selected = selectedMcpResourceIds(next);
        // Preserve a singular choice when switching to a single-server consumer.
        // Never silently choose one server from an ambiguous multi-selection.
        if (selected.length === 1) next.mcp_server = selected[0];
        else delete next.mcp_server;
        delete next.mcp_servers; changed = true;
      }
      if (next[kind] && !fieldOptions(kind).some((option) => option.value === next[kind])) {
        delete next[kind]; changed = true;
      }
    }
    if (changed) onChange(next);
  }, [fields, inventory, loading, multipleMcp, onChange, required, value]);

  const selectedRobot = value.robot
    ? (inventory.options.robot ?? []).find((option) => option.value === value.robot)
    : undefined;
  const selectedResourcesAvailable = !value.robot || selectedRobot?.selectable === true;
  const requiredFields = requiredAgentResourcePickerKinds(required);
  const pendingBlocksLaunch = [...pending].some((kind) =>
    requiredFields.includes(kind)
      || (kind === 'mcp_server' ? selectedMcpResourceIds(value).length > 0 : Boolean(value[kind]))
  );
  const needsScreenRecording = actions.has('computer/observe');
  const needsAccessibility = actions.has('computer/a11y.observe') || actions.has('computer/input');
  const computerAvailable = !computerRequired || Boolean(computerPermissions && (
    computerPermissions.platform === 'windows'
      || computerPermissions.platform === 'macos'
        && (!needsScreenRecording || computerPermissions.screen_recording === true)
        && (!needsAccessibility || computerPermissions.accessibility === true)
  ));
  const liveResourcesReady = !pendingBlocksLaunch && !computerLoading && !computerLoadFailed
    && computerAvailable && selectedResourcesAvailable;
  const displayedMissingCount = missingChoiceCount
    + (!computerLoading && !computerAvailable ? 1 : 0)
    + (value.robot && !selectedResourcesAvailable ? 1 : 0);
  useEffect(() => {
    onAvailabilityChange?.(liveResourcesReady);
    return () => onAvailabilityChange?.(false);
  }, [liveResourcesReady, onAvailabilityChange]);

  if (!fields.length && !computerRequired) return null;
  const kindName = (kind: string) => t(`agentSettings.resources.kinds.${kind === 'mcp_server' ? 'mcpConnection' : kind.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase())}`);
  const permissionName = (permission: keyof IApiRobotPermissions) =>
    t(`agentSettings.resources.robotPermissions.${permission}`);
  const optionDescription = (kind: UserAgentResourceKind, option: AgentResourceOption) => {
    if (kind !== 'robot' || !option.robotPhase) return option.description;
    const phase = t(`agentSettings.resources.robotPhases.${option.robotPhase}`);
    const disabledPermissions = option.robotDisabledPermissions ?? [];
    const unsupportedPermissions = option.robotUnsupportedPermissions ?? [];
    const requiredPermissions = option.robotRequiredPermissions ?? [];
    const authority = unsupportedPermissions.length
      ? t('agentSettings.resources.robotHardwareMissing', {
          permissions: unsupportedPermissions.map(permissionName).join(', '),
        })
      : disabledPermissions.length
      ? t('agentSettings.resources.robotPermissionsMissing', {
          permissions: disabledPermissions.map(permissionName).join(', '),
        })
      : requiredPermissions.length
        ? t('agentSettings.resources.robotPermissionsReady', {
            permissions: requiredPermissions.map(permissionName).join(', '),
          })
        : t('agentSettings.resources.robotPermissionsNotRequired');
    return [option.description, phase, authority].filter(Boolean).join(' · ');
  };
  return <section className={styles.picker} aria-label={t('agentSettings.resources.pickerTitle')}>
    <div className={styles.header}><div className={styles.headerCopy}><strong>{t('agentSettings.resources.pickerTitle')}</strong><span>{t(companionBindings ? 'agentSettings.resources.companionBindingHint' : 'agentSettings.resources.pickerHint')}</span></div>
      <span className={`${styles.status} ${!resolution.missingKinds.length && liveResourcesReady ? styles.statusReady : ''}`}>{t(resolution.missingKinds.length || !liveResourcesReady ? 'agentSettings.resources.missingCount' : 'agentSettings.resources.ready', { count: displayedMissingCount })}</span>
    </div>
    {computerRequired && <div className={styles.automaticStatus} role='status'>
      <div><strong>{kindName('computer')}</strong><span>{t(
        computerLoading
          ? 'agentSettings.resources.computerChecking'
          : computerLoadFailed
            ? 'agentSettings.resources.computerCheckFailed'
            : computerAvailable
              ? 'agentSettings.resources.computerReady'
              : 'agentSettings.resources.computerPermissionNeeded'
      )}</span></div>
      {!computerLoading && !computerAvailable && <Button size='mini' type='text' onClick={() => {
        if (onNavigateToResource) onNavigateToResource('/settings/computer-use');
        else void navigate('/settings/computer-use');
      }}>{t('agentSettings.resources.configure')}</Button>}
    </div>}
    <div className={styles.fields}>{fields.map((kind) => {
      const options = fieldOptions(kind);
      const loading = pending.has(kind);
      const error = inventory.errors[kind];
      const dependsOn = kind === 'channel' || kind === 'robot' ? (required.has('customer') ? 'customer' : required.has('companion') || required.has('companion_memory') ? 'companion' : undefined) : kind === 'knowledge_base' && required.has('customer') ? 'customer' : undefined;
      const waitingDependency = Boolean(dependsOn && !value[dependsOn as UserAgentResourceKind]);
      // Select owns an internal input. A wrapping label forwards a second native
      // click to it, toggling the popup closed immediately after it opens. Keep
      // the field non-labeling; the combobox has its own accessible aria-label.
      return <div className={styles.field} key={kind}><span className={styles.fieldLabel}>{kindName(kind)} {optionalKinds.includes(kind) && t('common.optional')}</span><div className={styles.fieldControl}>
        <Select allowClear={optionalKinds.includes(kind)} mode={kind === 'mcp_server' && multipleMcp ? 'multiple' : undefined} value={kind === 'mcp_server' && multipleMcp ? selectedMcpResourceIds(value) : value[kind]} disabled={disabled || loading || waitingDependency || options.length === 0} placeholder={t(waitingDependency ? 'agentSettings.resources.selectDependency' : 'agentSettings.resources.selectPlaceholder', { resource: dependsOn ? kindName(dependsOn as UserAgentResourceKind) : kindName(kind) })} aria-label={t('agentSettings.resources.selectAria', { resource: kindName(kind) })} onClear={() => { const next = { ...value }; delete next[kind]; if (kind === 'mcp_server') next.mcp_servers = []; onChange(next); }} onChange={(resourceId: string | string[]) => {
          if (kind === 'mcp_server' && multipleMcp) {
            const ids = Array.isArray(resourceId) ? resourceId : [resourceId];
            if (ids.length > 16) return;
            const next = { ...value, mcp_servers: ids }; delete next.mcp_server; onChange(next);
          } else if (typeof resourceId === 'string') {
            // Staged bindings belong to the previous companion. Clear them in
            // the same interaction, even if another inventory is still pending.
            if (kind === 'companion' && companionBindings && resourceId !== value.companion) {
              onChange({ companion: resourceId });
            } else onChange({ ...value, [kind]: resourceId });
          }
        }}>
          {options.map((option) => <Select.Option key={option.value} value={option.value} disabled={option.selectable === false}><span className={kind === 'companion' ? styles.companionOption : styles.option}>
            {kind === 'companion' && <span className={styles.optionAvatar} aria-hidden='true'><CompanionAvatar {...option.avatar} mood='content' activity='idle' size={24} /></span>}
            <strong title={option.label}>{option.label}</strong>{optionDescription(kind, option) && <small title={kind === 'companion' ? option.value : optionDescription(kind, option)}>{optionDescription(kind, option)}</small>}
          </span></Select.Option>)}
        </Select>
        {!loading && !waitingDependency && options.length === 0 && <Button className={styles.emptyButton} size='mini' type='text' icon={<LinkOne theme='outline' size={13} />} onClick={(event) => { event.preventDefault(); const route = routeForKind(kind, value); if (onNavigateToResource) onNavigateToResource(route); else void navigate(route); }}>{t('agentSettings.resources.configure')}</Button>}
      </div><span className={`${styles.fieldHint} ${error ? styles.fieldError : ''}`}>{loading ? <Spin size={10} /> : error ? t('agentSettings.resources.loadFailed') : waitingDependency ? t('agentSettings.resources.dependencyHint', { resource: kindName(dependsOn as UserAgentResourceKind) }) : options.length === 0 ? t('agentSettings.resources.emptyOptions') : t(kind === 'mcp_server' && companionBindings ? 'nomi.chat.mcpScopeHint' : kind === 'mcp_server' && multipleMcp ? 'agentSettings.resources.mcpSelectionHint' : 'agentSettings.resources.selectionHint')}</span></div>;
    })}</div>
  </section>;
};

export default AgentResourcePicker;
