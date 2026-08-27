import type { IKnowledgeFileEntry, IKnowledgeTreeEntry } from '@/common/adapter/ipcBridge';
import {
  hasKnowledgeEntryCapability,
  knowledgeTreeEntryFromFile,
} from './entryCapabilities';

function fileName(relPath: string): string {
  return relPath.split('/').filter(Boolean).at(-1) ?? relPath;
}

function dirNode(name: string, relPath: string): IKnowledgeTreeEntry {
  return {
    name,
    rel_path: relPath,
    is_dir: true,
    is_file: false,
    modified_at: null,
    children: [],
  };
}

function fileNode(
  file: IKnowledgeFileEntry,
  known?: IKnowledgeTreeEntry
): IKnowledgeTreeEntry {
  const projected = knowledgeTreeEntryFromFile(file);
  return {
    ...known,
    ...projected,
    name: fileName(file.rel_path),
  };
}

export function sortKnowledgeTreeNodes(nodes: IKnowledgeTreeEntry[]): IKnowledgeTreeEntry[] {
  return nodes
    .map((node) => (node.children ? { ...node, children: sortKnowledgeTreeNodes(node.children) } : node))
    .sort((a, b) => {
      if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
      return a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }) || a.name.localeCompare(b.name);
    });
}

export function collectKnowledgeDirectoryPaths(nodes: IKnowledgeTreeEntry[]): string[] {
  const paths: string[] = [];
  const visit = (items: IKnowledgeTreeEntry[]) => {
    for (const item of items) {
      if (!item.is_dir) continue;
      paths.push(item.rel_path);
      if (item.children?.length) visit(item.children);
    }
  };
  visit(nodes);
  return paths;
}

/** Directory picker projection that never exposes the moving directory as its own target. */
export function knowledgeDirectoryOnlyTree(
  nodes: IKnowledgeTreeEntry[],
  movingEntry?: IKnowledgeTreeEntry | null
): IKnowledgeTreeEntry[] {
  return nodes.flatMap((node) => {
    if (!node.is_dir) return [];
    if (!hasKnowledgeEntryCapability(node, 'accept_children')) return [];
    if (
      movingEntry?.is_dir &&
      (node.rel_path === movingEntry.rel_path ||
        node.rel_path.startsWith(`${movingEntry.rel_path}/`))
    ) {
      return [];
    }
    return [
      {
        ...node,
        ...(node.children
          ? { children: knowledgeDirectoryOnlyTree(node.children, movingEntry) }
          : {}),
      },
    ];
  });
}

export function buildKnowledgeSearchTree(
  files: IKnowledgeFileEntry[],
  query: string,
  knownTree: IKnowledgeTreeEntry[] = []
): IKnowledgeTreeEntry[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];

  const root: IKnowledgeTreeEntry[] = [];
  const dirs = new Map<string, IKnowledgeTreeEntry>();
  const knownByPath = new Map<string, IKnowledgeTreeEntry>();
  const collectKnown = (nodes: IKnowledgeTreeEntry[]) => {
    for (const node of nodes) {
      knownByPath.set(node.rel_path, node);
      if (node.children?.length) collectKnown(node.children);
    }
  };
  collectKnown(knownTree);

  for (const file of files) {
    if (!file.rel_path.toLowerCase().includes(q)) continue;

    const segments = file.rel_path.split('/').filter(Boolean);
    let level = root;
    let currentPath = '';
    for (let i = 0; i < segments.length - 1; i += 1) {
      currentPath = currentPath ? `${currentPath}/${segments[i]}` : segments[i];
      let dir = dirs.get(currentPath);
      if (!dir) {
        const known = knownByPath.get(currentPath);
        dir = known?.is_dir
          ? { ...known, children: [] }
          : dirNode(segments[i], currentPath);
        dirs.set(currentPath, dir);
        level.push(dir);
      }
      dir.children ??= [];
      level = dir.children;
    }
    level.push(fileNode(file, knownByPath.get(file.rel_path)));
  }

  return sortKnowledgeTreeNodes(root);
}

export function mergeKnowledgeTreeChildren(
  nodes: IKnowledgeTreeEntry[],
  parentRelPath: string,
  children: IKnowledgeTreeEntry[]
): IKnowledgeTreeEntry[] {
  return nodes.map((node) => {
    if (node.rel_path === parentRelPath && node.is_dir) {
      return { ...node, children };
    }
    if (node.children?.length) {
      return { ...node, children: mergeKnowledgeTreeChildren(node.children, parentRelPath, children) };
    }
    return node;
  });
}

export function preserveKnowledgeTreeChildren(
  nextNodes: IKnowledgeTreeEntry[],
  previousNodes: IKnowledgeTreeEntry[]
): IKnowledgeTreeEntry[] {
  const previousByPath = new Map<string, IKnowledgeTreeEntry>();
  const collect = (nodes: IKnowledgeTreeEntry[]) => {
    for (const node of nodes) {
      if (node.is_dir) previousByPath.set(node.rel_path, node);
      if (node.children?.length) collect(node.children);
    }
  };
  collect(previousNodes);

  const preserve = (nodes: IKnowledgeTreeEntry[]): IKnowledgeTreeEntry[] =>
    nodes.map((node) => {
      if (!node.is_dir) return node;
      const previous = previousByPath.get(node.rel_path);
      if (node.children?.length) {
        return { ...node, children: preserve(node.children) };
      }
      if (previous?.children) {
        return { ...node, children: previous.children };
      }
      return node;
    });

  return preserve(nextNodes);
}

export function firstKnowledgeFilePath(nodes: IKnowledgeTreeEntry[]): string | null {
  for (const node of nodes) {
    if (node.is_file) return node.rel_path;
    if (node.children?.length) {
      const found = firstKnowledgeFilePath(node.children);
      if (found) return found;
    }
  }
  return null;
}

export function parentDirOfKnowledgePath(relPath: string | null): string {
  if (!relPath) return '';
  const parts = relPath.split('/').filter(Boolean);
  if (parts.length <= 1) return '';
  return parts.slice(0, -1).join('/');
}

export function knowledgeFolderPathChain(relPath: string): string[] {
  const parts = relPath.split('/').filter(Boolean);
  return parts.map((_, index) => parts.slice(0, index + 1).join('/'));
}

export function isKnowledgePathWithin(path: string | null, folderPath: string): boolean {
  if (!path || !folderPath) return false;
  return path === folderPath || path.startsWith(`${folderPath}/`);
}

export function replaceKnowledgePathPrefix(path: string | null, oldPrefix: string, newPrefix: string): string | null {
  if (!path) return path;
  if (path === oldPrefix) return newPrefix;
  if (path.startsWith(`${oldPrefix}/`)) return `${newPrefix}${path.slice(oldPrefix.length)}`;
  return path;
}

/**
 * The tree is path-addressed today, so a move has to update every loaded view
 * of that path in one reducer transition. Keeping this shape here makes the
 * migration to stable `entry_id` identities mechanical: paths become mutable
 * locators while the reducer contract stays the same.
 */
export interface KnowledgeTreeViewState {
  files: IKnowledgeFileEntry[];
  treeData: IKnowledgeTreeEntry[];
  expandedTreeKeys: string[];
  selectedPath: string | null;
  selectedTreeKey: string | null;
  selectedFolderPath: string;
}

export type KnowledgeRelocationIssue = 'same-parent' | 'self' | 'descendant';

export type KnowledgeTreeViewAction =
  | { type: 'sync'; files: IKnowledgeFileEntry[]; tree: IKnowledgeTreeEntry[] }
  | { type: 'set-root'; tree: IKnowledgeTreeEntry[] }
  | { type: 'merge-children'; parentPath: string; children: IKnowledgeTreeEntry[] }
  | { type: 'replace-expanded'; paths: string[] }
  | { type: 'expand'; paths: string[] }
  | { type: 'select-file'; path: string }
  | { type: 'select-folder'; path: string }
  | { type: 'select-tree-key'; path: string | null }
  | { type: 'reset-base' }
  | { type: 'remove-path'; path: string; parentPath: string }
  | { type: 'relocated'; oldPath: string; newPath: string };

export const initialKnowledgeTreeViewState: KnowledgeTreeViewState = {
  files: [],
  treeData: [],
  expandedTreeKeys: [],
  selectedPath: null,
  selectedTreeKey: null,
  selectedFolderPath: '',
};

function knowledgePathName(path: string): string {
  return path.split('/').filter(Boolean).at(-1) ?? path;
}

export function knowledgeRelocationPath(sourcePath: string, destinationParentPath: string, newName?: string): string {
  const name = newName?.trim() || knowledgePathName(sourcePath);
  return destinationParentPath ? `${destinationParentPath}/${name}` : name;
}

export function knowledgeRelocationIssue(
  sourcePath: string,
  sourceIsDirectory: boolean,
  destinationParentPath: string,
  newName?: string
): KnowledgeRelocationIssue | null {
  const destinationPath = knowledgeRelocationPath(sourcePath, destinationParentPath, newName);
  if (destinationPath === sourcePath) return 'same-parent';
  if (!sourceIsDirectory) return null;
  if (destinationParentPath === sourcePath) return 'self';
  if (destinationParentPath.startsWith(`${sourcePath}/`)) return 'descendant';
  return null;
}

/** Monotonic per-base event gate; duplicate and out-of-order events are ignored. */
export function isNewerKnowledgeTreeRevision(
  lastRevision: number | null,
  nextRevision: number
): boolean {
  return lastRevision == null || nextRevision > lastRevision;
}

function rewriteKnowledgeTreeNodePath(
  node: IKnowledgeTreeEntry,
  oldPath: string,
  newPath: string
): IKnowledgeTreeEntry {
  const relPath = replaceKnowledgePathPrefix(node.rel_path, oldPath, newPath) ?? node.rel_path;
  return {
    ...node,
    name: node.rel_path === oldPath ? knowledgePathName(newPath) : node.name,
    rel_path: relPath,
    // Relocating a directory updates the projected locator/revision of every
    // descendant. Keep already-loaded descendants CAS-current as well.
    revision: node.revision == null ? undefined : node.revision + 1,
    children: node.children?.map((child) => rewriteKnowledgeTreeNodePath(child, oldPath, newPath)),
  };
}

function removeKnowledgeTreeNode(
  nodes: IKnowledgeTreeEntry[],
  sourcePath: string
): { nodes: IKnowledgeTreeEntry[]; removed: IKnowledgeTreeEntry | null } {
  let removed: IKnowledgeTreeEntry | null = null;
  const next: IKnowledgeTreeEntry[] = [];

  for (const node of nodes) {
    if (node.rel_path === sourcePath) {
      removed = node;
      continue;
    }
    if (!removed && node.children?.length) {
      const childResult = removeKnowledgeTreeNode(node.children, sourcePath);
      if (childResult.removed) {
        removed = childResult.removed;
        next.push({ ...node, children: childResult.nodes });
        continue;
      }
    }
    next.push(node);
  }

  return { nodes: next, removed };
}

function insertKnowledgeTreeNode(
  nodes: IKnowledgeTreeEntry[],
  destinationParentPath: string,
  movedNode: IKnowledgeTreeEntry
): IKnowledgeTreeEntry[] {
  if (!destinationParentPath) return sortKnowledgeTreeNodes([...nodes, movedNode]);

  return nodes.map((node) => {
    if (node.rel_path === destinationParentPath && node.is_dir) {
      // `undefined` means this branch is still lazy. Inventing a one-item
      // children array here marks it as loaded and hides every pre-existing
      // child until a hard refresh. Leave it lazy; the caller reloads/expands
      // the destination branch after the authoritative move succeeds.
      if (node.children === undefined) return node;
      return { ...node, children: sortKnowledgeTreeNodes([...node.children, movedNode]) };
    }
    if (!node.children?.length) return node;
    return {
      ...node,
      children: insertKnowledgeTreeNode(node.children, destinationParentPath, movedNode),
    };
  });
}

/** Move a loaded tree node without manufacturing descendants that were never loaded. */
export function relocateKnowledgeTreeNodes(
  nodes: IKnowledgeTreeEntry[],
  oldPath: string,
  newPath: string
): IKnowledgeTreeEntry[] {
  const { nodes: withoutSource, removed } = removeKnowledgeTreeNode(nodes, oldPath);
  if (!removed) return nodes;
  const rewritten = rewriteKnowledgeTreeNodePath(removed, oldPath, newPath);
  return insertKnowledgeTreeNode(withoutSource, parentDirOfKnowledgePath(newPath), rewritten);
}

export function applyKnowledgePathRelocation(
  state: KnowledgeTreeViewState,
  oldPath: string,
  newPath: string
): KnowledgeTreeViewState {
  if (!oldPath || !newPath || oldPath === newPath) return state;

  const destinationParent = parentDirOfKnowledgePath(newPath);
  const selectedPath = replaceKnowledgePathPrefix(state.selectedPath, oldPath, newPath);
  const selectedTreeKey = replaceKnowledgePathPrefix(state.selectedTreeKey, oldPath, newPath);
  const selectedFileMovedWhileSelected =
    state.selectedPath != null &&
    state.selectedTreeKey === state.selectedPath &&
    selectedPath !== state.selectedPath;
  return {
    files: state.files.map((file) => ({
      ...file,
      rel_path: replaceKnowledgePathPrefix(file.rel_path, oldPath, newPath) ?? file.rel_path,
    })),
    treeData: relocateKnowledgeTreeNodes(state.treeData, oldPath, newPath),
    expandedTreeKeys: [
      ...new Set([
        ...state.expandedTreeKeys.map(
          (path) => replaceKnowledgePathPrefix(path, oldPath, newPath) ?? path
        ),
        ...knowledgeFolderPathChain(destinationParent),
      ]),
    ],
    selectedPath,
    selectedTreeKey,
    selectedFolderPath: selectedFileMovedWhileSelected
      ? parentDirOfKnowledgePath(selectedPath)
      : replaceKnowledgePathPrefix(state.selectedFolderPath || null, oldPath, newPath) ?? '',
  };
}

export function knowledgeTreeViewReducer(
  state: KnowledgeTreeViewState,
  action: KnowledgeTreeViewAction
): KnowledgeTreeViewState {
  switch (action.type) {
    case 'sync': {
      // Do not interpret a temporarily missing path as a new selection. A
      // tree-changed event may be racing this snapshot after an external move;
      // switching to the first file here would destroy the active draft before
      // the old→new locator mapping arrives.
      const selectedPath = state.selectedPath ?? action.files[0]?.rel_path ?? null;
      return {
        ...state,
        files: action.files,
        treeData: preserveKnowledgeTreeChildren(action.tree, state.treeData),
        selectedPath,
        selectedTreeKey: state.selectedTreeKey ?? selectedPath,
      };
    }
    case 'set-root':
      return { ...state, treeData: action.tree };
    case 'merge-children':
      return {
        ...state,
        treeData: mergeKnowledgeTreeChildren(state.treeData, action.parentPath, action.children),
      };
    case 'replace-expanded':
      return { ...state, expandedTreeKeys: [...new Set(action.paths)] };
    case 'expand':
      return { ...state, expandedTreeKeys: [...new Set([...state.expandedTreeKeys, ...action.paths])] };
    case 'select-file':
      return {
        ...state,
        selectedPath: action.path,
        selectedTreeKey: action.path,
        selectedFolderPath: parentDirOfKnowledgePath(action.path),
      };
    case 'select-folder':
      return { ...state, selectedTreeKey: action.path, selectedFolderPath: action.path };
    case 'select-tree-key':
      return { ...state, selectedTreeKey: action.path };
    case 'reset-base':
      return initialKnowledgeTreeViewState;
    case 'remove-path': {
      const selectedInside = isKnowledgePathWithin(state.selectedPath, action.path);
      const treeSelectionInside = isKnowledgePathWithin(state.selectedTreeKey, action.path);
      const folderSelectionInside = isKnowledgePathWithin(state.selectedFolderPath || null, action.path);
      return {
        ...state,
        expandedTreeKeys: state.expandedTreeKeys.filter(
          (path) => !isKnowledgePathWithin(path, action.path)
        ),
        selectedPath: selectedInside ? null : state.selectedPath,
        selectedTreeKey: treeSelectionInside ? action.parentPath || null : state.selectedTreeKey,
        selectedFolderPath: folderSelectionInside ? action.parentPath : state.selectedFolderPath,
      };
    }
    case 'relocated':
      return applyKnowledgePathRelocation(state, action.oldPath, action.newPath);
  }
}
