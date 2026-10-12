import type { HostedApp } from '@/common/types/appHosting';
import { formatWorkpathDisplay, type WorkpathDisplay, type WorkpathNameMode } from './sidebarDisplayPreferences';
import { workpathKey } from './workpathKey';
import type { WorkpathNode } from './workpathTree';

/** Live owner metadata changes only the label, never grouping or path identity. */
export function withAppProjectNames(tree: WorkpathNode[], apps: Pick<HostedApp, 'directory' | 'name'>[] | undefined): WorkpathNode[] {
  if (!apps?.length) return tree;
  const names = new Map(apps.filter(app => app.name.trim()).map(app => [workpathKey(app.directory), app.name]));
  return tree.map(node => {
    const name = names.get(node.key);
    return name ? { ...node, displayName: name, applicationName: name } : node;
  });
}

export function formatProjectWorkpathDisplay(node: WorkpathNode, mode: WorkpathNameMode): WorkpathDisplay {
  if (!node.applicationName) return formatWorkpathDisplay(node.key, node.displayName, mode);
  return mode === 'full' || mode === 'folderWithPath'
    ? { kind: 'twoLine', primary: node.applicationName, secondary: node.key, tooltip: node.key }
    : { kind: 'single', primary: node.applicationName, tooltip: node.key };
}
