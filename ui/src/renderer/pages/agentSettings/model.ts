import type {
  AgentPresetDocument,
  AgentPresetDraft,
  CapabilityCatalogItem,
  CapabilityId,
  CapabilityPlacement,
  ChatRouteCandidate,
  ChatRouteRecord,
  ExactCatalogRef,
  OfficialPresetKey,
  OfficialPresetTemplate,
  PreviewDiagnostic,
  ResolveAgentPresetPreviewResponse,
  SaveAgentPresetRevisionResponse,
} from '@/common/types/agentPlatform';

export const TEMPLATE_I18N_PATH: Record<OfficialPresetKey, string> = {
  'chat.minimal': 'chat.minimal',
  'assistant.general': 'assistant.general',
  'coding.codex': 'coding.codex',
  'companion.default': 'companion.default',
  'robot.default': 'robot.default',
  'customer-service.default': 'customerService.default',
  'creative-studio.default': 'creativeStudio.default',
};

export const chatRouteCandidateKey = (candidate: ChatRouteCandidate): string =>
  `${candidate.model_route_id}@${candidate.model_route_revision}`;

/**
 * Reorder an exact route record after a friendly model choice. The selected
 * candidate remains byte-for-byte intact.
 */
export function selectChatRouteCandidate(
  record: ChatRouteRecord | null | undefined,
  candidateKey: string
): ChatRouteRecord | null {
  if (!record) return null;
  const candidates = [record.primary, ...record.failovers];
  const selected = candidates.find(
    (candidate) => chatRouteCandidateKey(candidate) === candidateKey
  );
  if (!selected) return null;

  return {
    ...record,
    primary: selected,
    failovers: candidates.filter(
      (candidate) => chatRouteCandidateKey(candidate) !== candidateKey
    ),
  };
}

export interface SaveDraftRevisionPorts {
  preview(draft: AgentPresetDraft): Promise<ResolveAgentPresetPreviewResponse>;
  save(
    draft: AgentPresetDraft,
    preview: ResolveAgentPresetPreviewResponse
  ): Promise<SaveAgentPresetRevisionResponse>;
}

export async function saveDraftRevisionWithPreview(
  draft: AgentPresetDraft,
  ports: SaveDraftRevisionPorts
): Promise<{
  preview: ResolveAgentPresetPreviewResponse;
  saved: SaveAgentPresetRevisionResponse | null;
}> {
  const preview = await ports.preview(draft);
  if (!preview.can_save_revision || preview.status !== 'ready') {
    return { preview, saved: null };
  }
  return {
    preview,
    saved: await ports.save(draft, preview),
  };
}

export function templateCapabilityCount(template: OfficialPresetTemplate): number {
  return template.seed.initial_capabilities.length + template.seed.on_demand_capabilities.length;
}

export function updateDocument(
  draft: AgentPresetDraft,
  transform: (document: AgentPresetDocument) => AgentPresetDocument
): AgentPresetDraft {
  return { ...draft, document: transform(draft.document) };
}

export const capabilityReferenceKey = (
  reference: ExactCatalogRef<'capability'>
): string => `${reference.id}@${reference.version}`;

export const RESOURCE_KIND_I18N_KEYS: Readonly<Record<string, string>> = {
  asset_library: 'assetLibrary',
  browser: 'browser',
  canvas: 'canvas',
  channel: 'channel',
  companion: 'companion',
  companion_memory: 'companionMemory',
  computer: 'computer',
  customer: 'customer',
  generation_provider: 'generationProvider',
  knowledge_base: 'knowledgeBase',
  mcp_server: 'mcpConnection',
  miniapp: 'miniApp',
  process_session: 'processSession',
  project_memory: 'projectMemory',
  robot: 'robot',
  ssh_host: 'sshHost',
  terminal: 'terminal',
  workspace: 'workspace',
};

export const humanizeResourceKind = (resourceKind: string): string =>
  resourceKind
    .split('_')
    .filter(Boolean)
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(' ');

const CAPABILITY_FAMILY_LABELS: Readonly<
  Record<string, { en: string; zh: string }>
> = {
  a11y: { en: 'Accessibility', zh: '无障碍' },
  agent: { en: 'Agent collaboration', zh: 'Agent 协作' },
  autowork: { en: 'AutoWork', zh: '自动工作' },
  browser: { en: 'Browser', zh: '浏览器' },
  channel: { en: 'Channel', zh: '渠道' },
  citation: { en: 'Citations', zh: '引用' },
  companion: { en: 'Companion', zh: '伙伴' },
  computer: { en: 'Computer', zh: '电脑控制' },
  connector: { en: 'Connector', zh: '连接器' },
  creation: { en: 'Creation', zh: '内容创作' },
  customer_service: { en: 'Customer service', zh: '客户服务' },
  fs: { en: 'Files', zh: '文件' },
  idmm: { en: 'Intelligent decisions', zh: '智能决策' },
  ingress: { en: 'Ingress', zh: '外部接入' },
  knowledge: { en: 'Knowledge', zh: '知识库' },
  llm: { en: 'Model', zh: '模型' },
  mcp: { en: 'MCP', zh: 'MCP' },
  memory: { en: 'Memory', zh: '记忆' },
  miniapp: { en: 'MiniApp', zh: '小程序' },
  notification: { en: 'Notifications', zh: '通知' },
  office: { en: 'Office', zh: '办公文档' },
  process: { en: 'Process', zh: '进程' },
  remote: { en: 'Remote access', zh: '远程访问' },
  requirements: { en: 'Requirements', zh: '需求' },
  robot: { en: 'Robot', zh: '机器人' },
  schedule: { en: 'Scheduling', zh: '定时任务' },
  session: { en: 'Session', zh: '会话' },
  skill: { en: 'Skills', zh: '技能' },
  ssh: { en: 'SSH', zh: 'SSH' },
  terminal: { en: 'Terminal', zh: '终端' },
  vcs: { en: 'Version control', zh: '版本控制' },
  web: { en: 'Web research', zh: 'Web 调研' },
  workshop: { en: 'Creative Studio', zh: '创意工坊' },
  workspace: { en: 'Workspace', zh: '工作区' },
};

const CAPABILITY_ACTION_LABELS: Readonly<
  Record<string, { en: string; zh: string }>
> = {
  act: { en: 'interaction', zh: '交互' },
  artifacts: { en: 'artifacts', zh: '产物' },
  asr: { en: 'speech recognition', zh: '语音识别' },
  attach: { en: 'attachment access', zh: '附件访问' },
  attachments: { en: 'attachment access', zh: '附件访问' },
  autogen: { en: 'auto-generation', zh: '自动生成' },
  bind: { en: 'resource binding', zh: '资源选择' },
  catalog: { en: 'catalog access', zh: '目录访问' },
  citation: { en: 'citations', zh: '引用' },
  claim: { en: 'claiming', zh: '认领' },
  commit: { en: 'commits', zh: '提交' },
  connect: { en: 'connection', zh: '连接' },
  data: { en: 'data access', zh: '数据访问' },
  delegate: { en: 'delegation', zh: '委派' },
  delete: { en: 'deletion', zh: '删除' },
  describe: { en: 'descriptions', zh: '说明' },
  diff: { en: 'diffs', zh: '差异' },
  distill: { en: 'distillation', zh: '提炼' },
  download: { en: 'downloads', zh: '下载' },
  edit: { en: 'editing', zh: '编辑' },
  embedding: { en: 'embeddings', zh: '向量化' },
  evaluate: { en: 'script evaluation', zh: '脚本执行' },
  evolve: { en: 'evolution', zh: '演进' },
  exec: { en: 'execution', zh: '执行' },
  fetch: { en: 'fetching', zh: '抓取' },
  fork: { en: 'forking', zh: '派生' },
  generate: { en: 'generation', zh: '生成' },
  group_policy: { en: 'group policy', zh: '群组策略' },
  handoff: { en: 'handoff', zh: '转交' },
  hooks: { en: 'turn hooks', zh: '回合钩子' },
  identity: { en: 'identity', zh: '身份' },
  input: { en: 'input control', zh: '输入控制' },
  intervene: { en: 'intervention', zh: '干预' },
  invoke: { en: 'invocation', zh: '调用' },
  launch: { en: 'launch', zh: '启动' },
  learn: { en: 'learning', zh: '学习' },
  link: { en: 'device link', zh: '设备连接' },
  merge: { en: 'merging', zh: '合并' },
  mount: { en: 'mounting', zh: '挂载' },
  motion: { en: 'motion control', zh: '运动控制' },
  navigate: { en: 'navigation', zh: '导航' },
  notes: { en: 'notes', zh: '备注' },
  observe: { en: 'observation', zh: '观察' },
  pairing: { en: 'pairing', zh: '配对' },
  patch: { en: 'patching', zh: '修改' },
  persona: { en: 'persona', zh: '角色设定' },
  plan: { en: 'planning', zh: '规划' },
  proxy: { en: 'proxy', zh: '代理' },
  publish: { en: 'publishing', zh: '发布' },
  push: { en: 'pushes', zh: '推送' },
  read: { en: 'reading', zh: '读取' },
  recall: { en: 'recall', zh: '回忆' },
  receive: { en: 'receiving', zh: '接收' },
  render: { en: 'rendering', zh: '渲染' },
  render_content: { en: 'content rendering', zh: '内容渲染' },
  reply: { en: 'replies', zh: '回复' },
  rerank: { en: 'reranking', zh: '重排序' },
  resource: { en: 'resource access', zh: '资源访问' },
  rest: { en: 'REST access', zh: 'REST 接入' },
  roster: { en: 'roster access', zh: '伙伴列表' },
  scratch: { en: 'scratch memory', zh: '临时记忆' },
  search: { en: 'search', zh: '搜索' },
  send: { en: 'sending', zh: '发送' },
  serve: { en: 'serving', zh: '提供服务' },
  session: { en: 'session access', zh: '会话访问' },
  site_memory: { en: 'site memory', zh: '站点记忆' },
  snapshot: { en: 'snapshots', zh: '快照' },
  stage: { en: 'staging', zh: '暂存' },
  status: { en: 'status', zh: '状态' },
  steer: { en: 'steering', zh: '调整执行' },
  store: { en: 'storage', zh: '存储' },
  sudo: { en: 'privileged execution', zh: '管理员执行' },
  sync: { en: 'synchronization', zh: '同步' },
  takeover: { en: 'takeover', zh: '接管' },
  template: { en: 'templates', zh: '模板' },
  timer: { en: 'timers', zh: '定时触发' },
  tool_proxy: { en: 'tool proxy', zh: '工具代理' },
  tts: { en: 'speech synthesis', zh: '语音合成' },
  upload: { en: 'uploads', zh: '上传' },
  video: { en: 'video', zh: '视频' },
  vision: { en: 'vision', zh: '视觉' },
  watch: { en: 'change watching', zh: '变化监听' },
  webhook: { en: 'webhook delivery', zh: 'Webhook 投递' },
  write: { en: 'writing', zh: '写入' },
};

const CAPABILITY_COPY_OVERRIDES: Readonly<
  Record<string, { en: [string, string]; zh: [string, string] }>
> = {
  'channel.group_policy': {
    en: ['Channel group policy', 'Manage group policy for the selected channel.'],
    zh: ['管理渠道群组策略', '管理当前使用目标所选渠道的群组策略。'],
  },
  'channel.pairing': {
    en: ['Pair a channel', 'Pair the Agent with an approved messaging channel.'],
    zh: ['配对消息渠道', '将 Agent 与已批准的消息渠道配对。'],
  },
  'channel.receive': {
    en: ['Receive channel messages', 'Receive inbound messages from the selected channel.'],
    zh: ['接收渠道消息', '接收当前使用目标所选渠道的消息。'],
  },
  'channel.reply': {
    en: ['Reply in a channel', 'Reply through the selected messaging channel.'],
    zh: ['回复渠道消息', '通过当前使用目标所选渠道回复消息。'],
  },
  'channel.send': {
    en: ['Send channel messages', 'Send outbound messages through the selected channel.'],
    zh: ['发送渠道消息', '通过当前使用目标所选渠道发送消息。'],
  },
  'companion.evolve': {
    en: ['Evolve the companion', 'Submit a bounded evolution action for the selected Companion.'],
    zh: ['演进伙伴', '对当前使用目标所选伙伴提交受限的演进操作。'],
  },
  'companion.learn': {
    en: ['Learn from a conversation', 'Submit a bounded learning action for the selected Companion.'],
    zh: ['让伙伴学习', '对当前使用目标所选伙伴提交受限的学习操作。'],
  },
  'companion.persona': {
    en: ['Companion persona', 'Provide the selected Companion persona as Agent context.'],
    zh: ['伙伴角色设定', '将当前使用目标所选伙伴的角色设定提供给 Agent。'],
  },
  'companion.roster': {
    en: ['Companion list', 'Show the Companions available to the current usage target.'],
    zh: ['伙伴列表', '显示当前使用目标可用的伙伴列表。'],
  },
  'knowledge.autogen': {
    en: ['Generate knowledge', 'Generate bounded material for the selected knowledge base.'],
    zh: ['生成知识内容', '为当前会话所选知识库生成受限内容。'],
  },
  'knowledge.read': {
    en: ['Read the knowledge base', 'Read documents from the knowledge base selected at use time.'],
    zh: ['读取知识库', '读取使用时由当前会话选择的知识库内容。'],
  },
  'knowledge.search': {
    en: ['Search the knowledge base', 'Search the knowledge base selected at use time.'],
    zh: ['搜索知识库', '搜索使用时由当前会话选择的知识库。'],
  },
  'knowledge.write': {
    en: ['Write to the knowledge base', 'Write bounded material to the knowledge base selected at use time.'],
    zh: ['写入知识库', '向使用时由当前会话选择的知识库写入受限内容。'],
  },
  'memory.companion.evolve': {
    en: ['Evolve companion memory', 'Apply a bounded evolution action to the selected Companion memory.'],
    zh: ['演进伙伴记忆', '对当前使用目标所选伙伴记忆执行受限的演进操作。'],
  },
  'memory.companion.merge': {
    en: ['Merge companion memory', 'Merge bounded material into the selected Companion memory.'],
    zh: ['合并伙伴记忆', '将受限内容合并到当前使用目标所选伙伴记忆。'],
  },
  'memory.companion.recall': {
    en: ['Read companion memory', 'Provide the selected Companion memory as Agent context.'],
    zh: ['读取伙伴记忆', '将当前使用目标所选伙伴记忆提供给 Agent。'],
  },
  'memory.companion.write': {
    en: ['Write companion memory', 'Write bounded material to the selected Companion memory.'],
    zh: ['写入伙伴记忆', '向当前使用目标所选伙伴记忆写入受限内容。'],
  },
  'memory.project.citation': {
    en: ['Cite project memory', 'Provide citations for project-memory entries selected by the usage target.'],
    zh: ['引用项目记忆', '为使用目标所选项目记忆条目提供引用。'],
  },
  'memory.project.distill': {
    en: ['Distill project memory', 'Distill bounded turn material into project memory.'],
    zh: ['提炼项目记忆', '将受限回合内容提炼到项目记忆中。'],
  },
  'memory.project.read': {
    en: ['Read project memory', 'Provide selected project-memory context to the Agent.'],
    zh: ['读取项目记忆', '将当前使用目标所选项目记忆提供给 Agent。'],
  },
  'memory.project.write': {
    en: ['Write project memory', 'Write bounded material to project memory.'],
    zh: ['写入项目记忆', '向项目记忆写入受限内容。'],
  },
  'memory.session.scratch': {
    en: ['Use session scratch memory', 'Use temporary memory scoped to the current Session.'],
    zh: ['使用会话临时记忆', '使用仅属于当前会话的临时记忆。'],
  },
  'session.attachments.read': {
    en: ['Read session attachments', 'Provide attachments already added to the current Session as context.'],
    zh: ['读取会话附件', '将当前会话已添加的附件提供给 Agent。'],
  },
  'workspace.artifacts': {
    en: ['Use workspace artifacts', 'Expose artifacts from the workspace selected at use time.'],
    zh: ['使用工作区产物', '提供使用时由当前会话选择的工作区产物。'],
  },
  'workspace.bind': {
    en: ['Choose a workspace', 'Allow the current usage target to choose a workspace for the Agent.'],
    zh: ['选择工作区', '允许当前使用目标为 Agent 选择工作区。'],
  },
};

const placeholderCapabilityCopy = (value: string, capabilityId: string): boolean => {
  const normalized = value.trim();
  return (
    normalized.length === 0 ||
    normalized === capabilityId ||
    /^bundled wave \d+ .*capability(?: contribution)?\.?$/i.test(normalized)
  );
};

const capabilityActionText = (parts: string[], language: string): string => {
  const key = parts.join('_');
  const exact = CAPABILITY_ACTION_LABELS[key];
  if (exact) return language.toLowerCase().startsWith('zh') ? exact.zh : exact.en;
  return parts
    .map((part) => {
      const known = CAPABILITY_ACTION_LABELS[part];
      return known
        ? language.toLowerCase().startsWith('zh')
          ? known.zh
          : known.en
        : part.replaceAll('_', ' ');
    })
    .join(language.toLowerCase().startsWith('zh') ? ' ' : ' ');
};

export const capabilityProductName = (
  capabilityId: string,
  language: string
): string => {
  const [family, ...parts] = capabilityId.split('.').filter(Boolean);
  const labels = CAPABILITY_FAMILY_LABELS[family];
  const familyLabel = labels
    ? language.toLowerCase().startsWith('zh')
      ? labels.zh
      : labels.en
    : family;
  const action = capabilityActionText(parts, language);
  if (!action) return familyLabel;
  if (language.toLowerCase().startsWith('zh')) {
    return `${familyLabel} · ${action}`;
  }
  return `${familyLabel}: ${action}`;
};

export const capabilityProductCopy = (
  item: CapabilityCatalogItem,
  language: string
): { name: string; description: string } => {
  const override = CAPABILITY_COPY_OVERRIDES[item.capability.id];
  const zh = language.toLowerCase().startsWith('zh');
  if (
    override &&
    (placeholderCapabilityCopy(item.display_name, item.capability.id) ||
      placeholderCapabilityCopy(item.description, item.capability.id))
  ) {
    const [name, description] = override[zh ? 'zh' : 'en'];
    return { name, description };
  }
  const name = placeholderCapabilityCopy(item.display_name, item.capability.id)
    ? capabilityProductName(item.capability.id, language)
    : item.display_name;
  if (!placeholderCapabilityCopy(item.description, item.capability.id)) {
    return { name, description: item.description };
  }

  const resourceHint =
    item.required_resource_kinds.length > 0
      ? zh
        ? '具体资源由当前会话或使用目标在使用时选择。'
        : 'The current conversation or usage target selects the concrete resource when it is used.'
      : '';
  const description =
    item.kind === 'context_contributor'
      ? zh
        ? `把${name}提供的信息加入 Agent 上下文。${resourceHint}`
        : `Adds ${name.toLowerCase()} information to the Agent context. ${resourceHint}`
      : item.kind === 'resource_provider'
        ? zh
          ? `声明 Agent 可以申请${name}所需的资源。${resourceHint}`
          : `Declares the resource boundary needed for ${name.toLowerCase()}. ${resourceHint}`
        : item.kind === 'event_source'
          ? zh
            ? `把${name}事件送入 Agent 会话。${resourceHint}`
            : `Delivers ${name.toLowerCase()} events into the Agent Session. ${resourceHint}`
          : zh
            ? `允许 Agent 使用${name}。${resourceHint}`
            : `Lets the Agent use ${name.toLowerCase()}. ${resourceHint}`;
  return { name, description: description.trim() };
};

export const capabilityMatchesSearch = (
  item: CapabilityCatalogItem,
  query: string,
  language: string
): boolean => {
  const normalizedQuery = query.trim().toLowerCase();
  if (!normalizedQuery) return true;
  const copy = capabilityProductCopy(item, language);
  return [
    copy.name,
    copy.description,
    item.display_name,
    item.description,
    item.capability.id,
    item.source_package.id,
    ...item.required_resource_kinds,
  ]
    .join(' ')
    .toLowerCase()
    .includes(normalizedQuery);
};

export function capabilityPlacement(
  document: AgentPresetDocument,
  capability: CapabilityId | ExactCatalogRef<'capability'>
): CapabilityPlacement {
  const capabilityId = typeof capability === 'string' ? capability : capability.id;
  const version = typeof capability === 'string' ? undefined : capability.version;
  const matches = (selection: AgentPresetDocument['initial_capabilities'][number]) =>
    selection.capability.id === capabilityId &&
    (version === undefined || selection.capability.version === version);

  if (document.initial_capabilities.some(matches)) {
    return 'initial';
  }
  if (document.on_demand_capabilities.some(matches)) {
    return 'on_demand';
  }
  return 'none';
}

/**
 * Capability placement is the complete authoring contract. Resource instances
 * are selected later by the conversation, companion, or automation target.
 */
export function placeCapability(
  document: AgentPresetDocument,
  capability: ExactCatalogRef<'capability'>,
  placement: CapabilityPlacement
): AgentPresetDocument {
  const existing = [...document.initial_capabilities, ...document.on_demand_capabilities].find(
    (item) =>
      item.capability.id === capability.id &&
      item.capability.version === capability.version
  );
  const selection = {
    capability,
    ...(existing?.action_allowlist?.length
      ? { action_allowlist: [...existing.action_allowlist] }
      : {}),
  };
  const without = (items: AgentPresetDocument['initial_capabilities']) =>
    items.filter((item) => item.capability.id !== capability.id);
  const initial = without(document.initial_capabilities);
  const onDemand = without(document.on_demand_capabilities);

  if (placement === 'initial') initial.push(selection);
  if (placement === 'on_demand') onDemand.push(selection);

  return {
    ...document,
    initial_capabilities: initial.sort((left, right) =>
      left.capability.id.localeCompare(right.capability.id)
    ),
    on_demand_capabilities: onDemand.sort((left, right) =>
      left.capability.id.localeCompare(right.capability.id)
    ),
  };
}

export function selectedRequiredResourceKinds(
  document: AgentPresetDocument,
  capabilities: readonly CapabilityCatalogItem[]
): string[] {
  const selectedReferences = new Set(
    [...document.initial_capabilities, ...document.on_demand_capabilities].map(
      (selection) => capabilityReferenceKey(selection.capability)
    )
  );
  const kinds = new Set<string>();
  for (const capability of capabilities) {
    if (!selectedReferences.has(capabilityReferenceKey(capability.capability))) continue;
    capability.required_resource_kinds.forEach((kind) => kinds.add(kind));
  }
  return [...kinds].sort();
}

export function sortCapabilitiesByPlacement(
  document: AgentPresetDocument,
  capabilities: readonly CapabilityCatalogItem[]
): CapabilityCatalogItem[] {
  const placementRank: Record<CapabilityPlacement, number> = {
    initial: 0,
    on_demand: 1,
    none: 2,
  };
  return [...capabilities].sort((left, right) => {
    const rankDifference =
      placementRank[capabilityPlacement(document, left.capability)] -
      placementRank[capabilityPlacement(document, right.capability)];
    return (
      rankDifference ||
      left.display_name.localeCompare(right.display_name) ||
      left.capability.id.localeCompare(right.capability.id)
    );
  });
}

export function editorCapabilityReferences(
  document: AgentPresetDocument,
  visibleCapabilities: readonly CapabilityCatalogItem[],
  allCapabilities: readonly CapabilityCatalogItem[]
): ExactCatalogRef<'capability'>[] {
  const catalogKeys = new Set(
    allCapabilities.map((item) => capabilityReferenceKey(item.capability))
  );
  const references = new Map<string, ExactCatalogRef<'capability'>>();
  for (const item of visibleCapabilities) {
    references.set(capabilityReferenceKey(item.capability), item.capability);
  }
  const selected = [
    ...document.initial_capabilities,
    ...document.on_demand_capabilities,
  ].map((selection) => selection.capability);
  for (const reference of selected) {
    const key = capabilityReferenceKey(reference);
    if (!catalogKeys.has(key)) references.set(key, reference);
  }
  const placementRank: Record<CapabilityPlacement, number> = {
    initial: 0,
    on_demand: 1,
    none: 2,
  };
  return [...references.values()].sort((left, right) => {
    const placementDifference =
      placementRank[capabilityPlacement(document, left)] -
      placementRank[capabilityPlacement(document, right)];
    return (
      placementDifference ||
      left.id.localeCompare(right.id) ||
      left.version.localeCompare(right.version)
    );
  });
}

export type AgentUiOperation =
  | 'load'
  | 'open'
  | 'create'
  | 'delete'
  | 'fork'
  | 'preview'
  | 'save'
  | 'test'
  | 'session-load'
  | 'turn'
  | 'session-fork'
  | 'session-delete';

export type AgentUiErrorKind =
  | 'route-unavailable'
  | 'network'
  | 'timeout'
  | 'preset-not-found'
  | 'session-deleted'
  | 'session-not-found'
  | 'snapshot-unavailable'
  | 'resource'
  | 'model'
  | 'conflict'
  | 'runtime'
  | 'unknown';

type ErrorShape = {
  code?: unknown;
  status?: unknown;
  kind?: unknown;
};

const errorShape = (error: unknown): ErrorShape | null =>
  error && typeof error === 'object' ? (error as ErrorShape) : null;

const errorCode = (error: unknown): string => {
  const code = errorShape(error)?.code;
  return typeof code === 'string' ? code.toUpperCase() : '';
};

const errorStatus = (error: unknown): number | null => {
  const status = errorShape(error)?.status;
  return typeof status === 'number' && Number.isFinite(status) ? status : null;
};

const errorKind = (error: unknown): string => {
  const kind = errorShape(error)?.kind;
  return typeof kind === 'string' ? kind.toLowerCase() : '';
};

export function classifyAgentUiError(
  error: unknown,
  operation: AgentUiOperation
): AgentUiErrorKind {
  const code = errorCode(error);
  const status = errorStatus(error);
  const kind = errorKind(error);

  if (code === 'AGENT_PRESET_NOT_FOUND') return 'preset-not-found';
  if (code === 'SESSION_DELETED') return 'session-deleted';
  if (code === 'SESSION_NOT_FOUND' || code === 'REMOTE_SESSION_NOT_FOUND') {
    return 'session-not-found';
  }
  if (code === 'SNAPSHOT_EXECUTOR_UNAVAILABLE') return 'snapshot-unavailable';
  if (
    code === 'PRESET_RESOURCE_NOT_BOUND' ||
    code === 'RESOURCE_OWNER_MISMATCH' ||
    code === 'CAPABILITY_RESOURCE_NOT_BOUND'
  ) {
    return 'resource';
  }
  if (
    code === 'MODEL_ROUTE_RECORD_INVALID' ||
    code === 'MODEL_ROUTE_NOT_FOUND' ||
    code === 'CAPABILITY_NOT_MATERIALIZED' ||
    code === 'CAPABILITY_UNAVAILABLE' ||
    code === 'CAPABILITY_UNAVAILABLE_ON_PLATFORM'
  ) {
    return 'model';
  }
  if (
    code === 'PRESET_REVISION_DIGEST_MISMATCH' ||
    code === 'IDEMPOTENCY_CONFLICT' ||
    status === 409
  ) {
    return 'conflict';
  }
  if (
    code === 'AGENT_PLATFORM_RUNTIME_FAILED' ||
    code === 'AGENT_PLATFORM_INTERNAL' ||
    code === 'REMOTE_OPEN_FAILED'
  ) {
    return 'runtime';
  }
  if (kind === 'timeout') return 'timeout';
  if (kind === 'network') return 'network';

  if (
    status === 404 ||
    status === 405 ||
    code === 'NON_JSON_RESPONSE' ||
    code === 'ROUTE_NOT_FOUND'
  ) {
    return 'route-unavailable';
  }
  if (operation === 'load' && status == null) return 'route-unavailable';
  return 'unknown';
}

export function agentUiErrorMessage(
  error: unknown,
  operation: AgentUiOperation
): string {
  switch (classifyAgentUiError(error, operation)) {
    case 'route-unavailable':
      return 'The current Nomi-core build does not expose the canonical Agent workflow. Start the matching service or update the application, then retry.';
    case 'network':
      return 'The Nomi-core service could not be reached. Check that the desktop service is running, then retry.';
    case 'timeout':
      return 'The Nomi-core service did not respond before the deadline. Retry once; if the result is uncertain, inspect the existing Session before submitting again.';
    case 'preset-not-found':
      return 'This Agent no longer exists. Reload the Agent Workbench.';
    case 'session-deleted':
      return 'This Session was deleted and can no longer be continued.';
    case 'session-not-found':
      return 'This Session is no longer available. Return to the Agent Workbench and choose another setup.';
    case 'snapshot-unavailable':
      return 'This saved setup cannot run on the current runtime. Its history is read-only; create a new Session from the current setup.';
    case 'resource':
      return 'The launch context cannot satisfy one of this Agent\'s declared resource requirements.';
    case 'model':
      return 'Choose an available Chat model before saving or testing this setup.';
    case 'conflict':
      return 'This setup changed elsewhere. Reload it before saving again.';
    case 'runtime':
      return 'Nomi-core could not start this operation. Check the selected model and capabilities, then retry.';
    case 'unknown':
    default:
      return 'The Agent operation could not be completed. Review the selected model and capabilities, then retry.';
  }
}

export function previewDiagnosticMessage(diagnostic: PreviewDiagnostic): string {
  switch (diagnostic.code.toUpperCase()) {
    case 'PRESET_RESOURCE_NOT_BOUND':
    case 'RESOURCE_OWNER_MISMATCH':
    case 'CAPABILITY_RESOURCE_NOT_BOUND':
      return 'The current launch context cannot satisfy a declared resource requirement.';
    case 'MODEL_ROUTE_RECORD_INVALID':
    case 'MODEL_ROUTE_NOT_FOUND':
      return 'Choose an available Chat model before continuing.';
    case 'CAPABILITY_NOT_MATERIALIZED':
    case 'CAPABILITY_UNAVAILABLE':
    case 'CAPABILITY_UNAVAILABLE_ON_PLATFORM':
      return 'One selected capability is unavailable on this installation.';
    case 'PRESET_REVISION_DIGEST_MISMATCH':
      return 'This setup changed while it was open. Reload it before saving.';
    case 'SNAPSHOT_EXECUTOR_UNAVAILABLE':
      return 'This setup is read-only on the current runtime. Fork a new Session to continue.';
    case 'ROLE_COVERAGE_INCOMPLETE':
    case 'CODING_CODEX_NATIVE_INCOMPLETE':
      return 'This setup requires a runtime feature that is not available on this host.';
    default:
      return diagnostic.severity === 'warning'
        ? 'An optional part of this setup is unavailable on the current host.'
        : 'This setup cannot be executed on the current host.';
  }
}
