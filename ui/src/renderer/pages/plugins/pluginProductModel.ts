import type {
  GeneratedPluginDraft,
  PluginLibraryResponse,
  PluginLifecycle,
  PluginProjectSummary,
  PluginSummary,
} from '@/common/types/pluginPlatform';

export type PluginProductCategory =
  | 'knowledge'
  | 'automation'
  | 'development'
  | 'integration'
  | 'system'
  | 'other';

export type PluginProductStatus = 'enabled' | 'disabled' | 'draft' | 'attention';

export interface PluginProductItem {
  key: string;
  displayName: string;
  description: string;
  category: PluginProductCategory;
  status: PluginProductStatus;
  mount?: PluginSummary;
  project?: PluginProjectSummary;
  contributionCount: number;
  updatedAtMs: number;
}

const categorySignals: Array<[PluginProductCategory, RegExp]> = [
  ['development', /代码|开发|仓库|git|build|deploy|debug|code|repository|terminal/i],
  ['automation', /自动|定时|流程|任务|通知|提醒|automation|schedule|workflow|trigger/i],
  ['integration', /连接|同步|消息|客服|渠道|api|webhook|connect|integration|database/i],
  ['knowledge', /知识|文档|网页|摘要|搜索|阅读|翻译|内容|knowledge|document|search|summary/i],
  ['system', /系统|文件|进程|浏览器|电脑|system|filesystem|process|browser/i],
];

export function pluginProductCategory(value: string): PluginProductCategory {
  return categorySignals.find(([, pattern]) => pattern.test(value))?.[0] ?? 'other';
}

function mountStatus(lifecycle: PluginLifecycle): PluginProductStatus {
  if (lifecycle === 'enabled') return 'enabled';
  if (lifecycle === 'disabled' || lifecycle === 'uninstalled_data_retained') return 'disabled';
  return 'attention';
}

export function pluginProductItems(library: PluginLibraryResponse): PluginProductItem[] {
  const linkedProjects = new Map(
    library.projects
      .filter((project) => project.linked_mount_id)
      .map((project) => [project.linked_mount_id as string, project])
  );
  const items: PluginProductItem[] = library.plugins.map((mount) => {
    const project = linkedProjects.get(mount.mount_id);
    const searchable = `${mount.display_name} ${mount.description ?? ''} ${mount.current?.package_id ?? ''}`;
    return {
      key: `mount:${mount.mount_id}`,
      displayName: mount.display_name,
      description: mount.description ?? '',
      category: pluginProductCategory(searchable),
      status: mountStatus(mount.lifecycle),
      mount,
      project,
      contributionCount: mount.contribution_count,
      updatedAtMs: Math.max(mount.updated_at_ms, project?.updated_at_ms ?? 0),
    };
  });
  for (const project of library.projects) {
    if (project.linked_mount_id) continue;
    const searchable = `${project.display_name} ${project.description ?? ''}`;
    items.push({
      key: `project:${project.project_id}`,
      displayName: project.display_name,
      description: project.description ?? '',
      category: pluginProductCategory(searchable),
      status: 'draft',
      project,
      contributionCount: 0,
      updatedAtMs: project.updated_at_ms,
    });
  }
  return items.sort((left, right) => right.updatedAtMs - left.updatedAtMs);
}

export function pluginPackageId(now = Date.now()): string {
  return `user.nomifun.plugin-${now.toString(36)}`;
}

export function pluginDraftSourceEdits(draft: GeneratedPluginDraft) {
  const packageJson = `${JSON.stringify({
    name: draft.package_id,
    version: draft.package_version,
    description: draft.description,
    private: true,
    type: 'module',
    dependencies: draft.dependencies,
  }, null, 2)}\n`;
  return [
    { path: 'nomifun.plugin.json', content: draft.manifest_content },
    { path: 'package.json', content: packageJson },
    { path: draft.source_path, content: draft.source_content },
  ] as const;
}

export function pluginProductMatches(item: PluginProductItem, query: string): boolean {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return true;
  return `${item.displayName} ${item.description} ${item.mount?.current?.package_id ?? ''}`
    .toLocaleLowerCase()
    .includes(normalized);
}
