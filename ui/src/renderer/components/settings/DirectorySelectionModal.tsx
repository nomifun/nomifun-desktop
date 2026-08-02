/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Input, Message, Modal, Spin } from '@arco-design/web-react';
import {
  IconCheck,
  IconClose,
  IconDown,
  IconFile,
  IconFolder,
  IconFolderAdd,
  IconRight,
} from '@arco-design/web-react/icon';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getBaseUrl } from '@/common/adapter/httpBridge';
import styles from './DirectorySelectionModal.module.css';

interface DirectoryItem {
  name: string;
  path: string;
  isDirectory: boolean;
  isFile?: boolean;
}

interface DirectoryData {
  items: DirectoryItem[];
  currentPath: string;
  canGoUp: boolean;
  parentPath?: string;
  truncated?: boolean;
  isRoot?: boolean;
}

interface DirectoryTreeNode extends DirectoryItem {
  children?: DirectoryTreeNode[];
  expanded?: boolean;
  loading?: boolean;
  error?: string;
  truncated?: boolean;
  virtual?: boolean;
}

interface DirectorySelectionModalProps {
  visible: boolean;
  isFileMode?: boolean;
  onConfirm: (paths: string[] | undefined) => void;
  onCancel: () => void;
}

interface ApiEnvelope<T> {
  data?: T;
  error?: string;
}

const sortNodes = (nodes: DirectoryTreeNode[]): DirectoryTreeNode[] =>
  [...nodes].sort((left, right) => {
    if (left.isDirectory !== right.isDirectory) return left.isDirectory ? -1 : 1;
    return left.name.localeCompare(right.name);
  });

const toTreeNodes = (items: DirectoryItem[]): DirectoryTreeNode[] => sortNodes(items.map((item) => ({ ...item })));

const pathLabel = (path: string): string => {
  const normalized = path.replace(/[\\/]+$/, '');
  return normalized.split(/[\\/]/).pop() || path;
};

const updateNodeByPath = (
  nodes: DirectoryTreeNode[],
  path: string,
  update: (node: DirectoryTreeNode) => DirectoryTreeNode
): DirectoryTreeNode[] =>
  nodes.map((node) => {
    if (!node.virtual && node.path === path) return update(node);
    if (!node.children) return node;
    const children = updateNodeByPath(node.children, path, update);
    return children === node.children ? node : { ...node, children };
  });

const findNodeByPath = (nodes: DirectoryTreeNode[], path: string): DirectoryTreeNode | undefined => {
  for (const node of nodes) {
    if (!node.virtual && node.path === path) return node;
    if (node.children) {
      const child = findNodeByPath(node.children, path);
      if (child) return child;
    }
  }
  return undefined;
};

const responseError = (rawText: string, status: number): Error => {
  let message = '';
  try {
    const parsed = rawText ? (JSON.parse(rawText) as ApiEnvelope<unknown>) : null;
    message = typeof parsed?.error === 'string' ? parsed.error : '';
  } catch {
    message = rawText.slice(0, 300);
  }
  return new Error(message || `HTTP ${status}`);
};

const requestData = async <T,>(url: string, init?: RequestInit): Promise<T> => {
  const response = await fetch(url, {
    credentials: 'include',
    cache: 'no-store',
    ...init,
  });
  const rawText = await response.text().catch(() => '');
  if (!response.ok) throw responseError(rawText, response.status);

  const envelope: ApiEnvelope<T> | T = rawText ? JSON.parse(rawText) : ({} as T);
  if (envelope && typeof envelope === 'object' && 'data' in envelope) {
    return (envelope as ApiEnvelope<T>).data as T;
  }
  return envelope as T;
};

const DirectorySelectionModal: React.FC<DirectorySelectionModalProps> = ({
  visible,
  isFileMode = false,
  onConfirm,
  onCancel,
}) => {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [treeNodes, setTreeNodes] = useState<DirectoryTreeNode[]>([]);
  const [selectedPath, setSelectedPath] = useState('');
  const [currentPath, setCurrentPath] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [newFolderParentPath, setNewFolderParentPath] = useState<string | null>(null);
  const [newFolderName, setNewFolderName] = useState('');
  const [creatingFolder, setCreatingFolder] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const initialRequestRef = useRef(0);

  const loadDirectory = useCallback(
    async (dirPath = ''): Promise<DirectoryData> => {
      const showFiles = isFileMode ? 'true' : 'false';
      const data = await requestData<DirectoryData>(
        `${getBaseUrl()}/api/fs/browse?path=${encodeURIComponent(dirPath)}&showFiles=${showFiles}`
      );
      if (!data || !Array.isArray(data.items)) throw new Error('Invalid response from server');
      return data;
    },
    [isFileMode]
  );

  const loadInitialTree = useCallback(async (dirPath = '') => {
    const requestId = initialRequestRef.current + 1;
    initialRequestRef.current = requestId;
    setLoading(true);
    setError(null);
    try {
      const data = await loadDirectory(dirPath);
      if (initialRequestRef.current !== requestId) return;
      setCurrentPath(data.currentPath || '');
      const visibleRoots: DirectoryTreeNode[] = data.currentPath
        ? [
            {
              name: pathLabel(data.currentPath),
              path: data.currentPath,
              isDirectory: true,
              isFile: false,
              expanded: true,
              children: toTreeNodes(data.items),
              truncated: data.truncated,
            },
          ]
        : toTreeNodes(data.items);
      setTreeNodes([
        {
          name: t('fileSelection.allFiles'),
          path: '',
          isDirectory: true,
          isFile: false,
          virtual: true,
          expanded: true,
          children: visibleRoots,
        },
      ]);
    } catch (loadError) {
      if (initialRequestRef.current === requestId) {
        setError(loadError instanceof Error ? loadError.message : String(loadError));
      }
    } finally {
      if (initialRequestRef.current === requestId) setLoading(false);
    }
  }, [loadDirectory, t]);

  useEffect(() => {
    if (!visible) return undefined;

    setSelectedPath('');
    setCurrentPath('');
    setTreeNodes([]);
    setNewFolderParentPath(null);
    setNewFolderName('');
    setCreateError(null);
    void loadInitialTree();

    return () => {
      initialRequestRef.current += 1;
    };
  }, [visible, loadInitialTree]);

  const loadNodeChildren = useCallback(
    async (path: string): Promise<boolean> => {
      setTreeNodes((nodes) =>
        updateNodeByPath(nodes, path, (node) => ({ ...node, expanded: true, loading: true, error: undefined }))
      );
      try {
        const data = await loadDirectory(path);
        setTreeNodes((nodes) =>
          updateNodeByPath(nodes, path, (node) => ({
            ...node,
            expanded: true,
            loading: false,
            children: toTreeNodes(data.items),
            truncated: data.truncated,
            error: undefined,
          }))
        );
        return true;
      } catch (loadError) {
        const message = loadError instanceof Error ? loadError.message : String(loadError);
        setTreeNodes((nodes) =>
          updateNodeByPath(nodes, path, (node) => ({ ...node, expanded: true, loading: false, error: message }))
        );
        return false;
      }
    },
    [loadDirectory]
  );

  const canSelect = useCallback(
    (item: DirectoryTreeNode) => !item.virtual && (isFileMode ? item.isFile === true : item.isDirectory),
    [isFileMode]
  );

  const handleToggleDirectory = useCallback(
    (item: DirectoryTreeNode) => {
      if ((!item.isDirectory && !item.virtual) || item.loading) return;
      if (item.expanded) {
        setTreeNodes((nodes) =>
          item.virtual
            ? nodes.map((node) => (node.virtual ? { ...node, expanded: false } : node))
            : updateNodeByPath(nodes, item.path, (node) => ({ ...node, expanded: false }))
        );
        return;
      }
      if (item.virtual) {
        setTreeNodes((nodes) => nodes.map((node) => (node.virtual ? { ...node, expanded: true } : node)));
      } else if (item.children) {
        setTreeNodes((nodes) => updateNodeByPath(nodes, item.path, (node) => ({ ...node, expanded: true })));
      } else {
        void loadNodeChildren(item.path);
      }
    },
    [loadNodeChildren]
  );

  const handleSelect = useCallback((path: string) => {
    setSelectedPath(path);
    setNewFolderParentPath(null);
    setNewFolderName('');
    setCreateError(null);
  }, []);

  const selectedNode = useMemo(() => findNodeByPath(treeNodes, selectedPath), [treeNodes, selectedPath]);
  const canCreateFolder = !isFileMode && Boolean(selectedNode?.isDirectory);

  const handleStartCreateFolder = useCallback(async () => {
    if (!selectedNode?.isDirectory) return;
    setCreateError(null);
    if (!selectedNode.children) {
      const loaded = await loadNodeChildren(selectedNode.path);
      if (!loaded) return;
    } else if (!selectedNode.expanded) {
      setTreeNodes((nodes) =>
        updateNodeByPath(nodes, selectedNode.path, (node) => ({ ...node, expanded: true }))
      );
    }
    setNewFolderName('');
    setNewFolderParentPath(selectedNode.path);
  }, [loadNodeChildren, selectedNode]);

  const handleCancelCreateFolder = useCallback(() => {
    setNewFolderParentPath(null);
    setNewFolderName('');
    setCreateError(null);
  }, []);

  const handleCreateFolder = useCallback(async () => {
    const parentPath = newFolderParentPath;
    const name = newFolderName.trim();
    if (!parentPath || creatingFolder) return;
    if (!name) {
      setCreateError(t('fileSelection.folderNameRequired'));
      return;
    }

    setCreatingFolder(true);
    setCreateError(null);
    try {
      const created = await requestData<DirectoryItem>(`${getBaseUrl()}/api/fs/directory`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ parentPath, name }),
      });
      setTreeNodes((nodes) =>
        updateNodeByPath(nodes, parentPath, (node) => ({
          ...node,
          expanded: true,
          children: sortNodes([
            ...(node.children || []).filter((child) => child.path !== created.path),
            { ...created },
          ]),
        }))
      );
      setSelectedPath(created.path);
      setNewFolderParentPath(null);
      setNewFolderName('');
      Message.success(t('fileSelection.createFolderSuccess'));
    } catch (createFolderError) {
      setCreateError(createFolderError instanceof Error ? createFolderError.message : String(createFolderError));
    } finally {
      setCreatingFolder(false);
    }
  }, [creatingFolder, newFolderName, newFolderParentPath, t]);

  const handleConfirm = () => {
    if (selectedPath) onConfirm([selectedPath]);
  };

  const renderNewFolderRow = (depth: number) => (
    <div
      className={`${styles.treeRow} ${styles.newFolderRow} ${createError ? styles.newFolderRowError : ''}`}
      style={{ paddingLeft: 12 + depth * 20 }}
    >
      <span className={styles.chevronPlaceholder} />
      <IconFolder className={styles.folderIcon} />
      <div className={styles.newFolderInputWrap}>
        <Input
          autoFocus
          size='small'
          value={newFolderName}
          placeholder={t('fileSelection.newFolderName')}
          status={createError ? 'error' : undefined}
          disabled={creatingFolder}
          onChange={(value) => {
            setNewFolderName(value);
            setCreateError(null);
          }}
          onPressEnter={() => void handleCreateFolder()}
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.stopPropagation();
              handleCancelCreateFolder();
            }
          }}
        />
        {createError && <span className={styles.inlineError}>{createError}</span>}
      </div>
      <button
        type='button'
        className={styles.inlineAction}
        aria-label={t('fileSelection.createFolder')}
        disabled={creatingFolder}
        onClick={() => void handleCreateFolder()}
      >
        <IconCheck />
      </button>
      <button
        type='button'
        className={styles.inlineAction}
        aria-label={t('common.cancel')}
        disabled={creatingFolder}
        onClick={handleCancelCreateFolder}
      >
        <IconClose />
      </button>
    </div>
  );

  const renderNode = (item: DirectoryTreeNode, depth: number): React.ReactNode => {
    const isDirectory = item.isDirectory || item.virtual;
    const selectable = canSelect(item);
    const selected = selectable && selectedPath === item.path;
    const childrenVisible = isDirectory && item.expanded;
    const key = item.virtual ? '__all_files__' : item.path;

    return (
      <React.Fragment key={key}>
        <div
          role='treeitem'
          aria-expanded={isDirectory ? Boolean(item.expanded) : undefined}
          aria-selected={selectable ? selected : undefined}
          tabIndex={0}
          title={item.virtual ? undefined : item.path}
          className={`${styles.treeRow} ${item.virtual ? styles.rootRow : ''} ${selected ? styles.selectedRow : ''}`}
          style={{ paddingLeft: 12 + depth * 20 }}
          onClick={() => {
            if (selectable) handleSelect(item.path);
            else if (item.virtual) handleToggleDirectory(item);
          }}
          onDoubleClick={() => {
            if (isDirectory && !item.virtual) handleToggleDirectory(item);
          }}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              if (selectable) handleSelect(item.path);
              else if (item.virtual) handleToggleDirectory(item);
            } else if (event.key === 'ArrowRight' && isDirectory && !item.expanded) {
              event.preventDefault();
              handleToggleDirectory(item);
            } else if (event.key === 'ArrowLeft' && isDirectory && item.expanded) {
              event.preventDefault();
              handleToggleDirectory(item);
            }
          }}
        >
          {isDirectory ? (
            <button
              type='button'
              className={styles.chevronButton}
              aria-label={item.expanded ? t('common.collapse') : t('common.expand')}
              onClick={(event) => {
                event.stopPropagation();
                handleToggleDirectory(item);
              }}
            >
              {item.loading ? (
                <span className={styles.nodeSpinner} />
              ) : item.expanded ? (
                <IconDown />
              ) : (
                <IconRight />
              )}
            </button>
          ) : (
            <span className={styles.chevronPlaceholder} />
          )}
          {isDirectory ? <IconFolder className={styles.folderIcon} /> : <IconFile className={styles.fileIcon} />}
          <span className={styles.nodeName}>{item.name}</span>
        </div>

        {childrenVisible && (
          <div role='group'>
            {newFolderParentPath === item.path && renderNewFolderRow(depth + 1)}
            {item.children?.map((child) => renderNode(child, depth + 1))}
            {!item.loading && item.children?.length === 0 && newFolderParentPath !== item.path && (
              <div className={styles.emptyNode} style={{ paddingLeft: 48 + depth * 20 }}>
                {t('fileSelection.emptyFolder')}
              </div>
            )}
            {item.error && (
              <div className={styles.nodeError} style={{ paddingLeft: 48 + depth * 20 }}>
                <span>{item.error}</span>
                <Button size='mini' type='text' onClick={() => void loadNodeChildren(item.path)}>
                  {t('common.retry')}
                </Button>
              </div>
            )}
            {item.truncated && (
              <div className={styles.truncated} style={{ paddingLeft: 48 + depth * 20 }}>
                {t('fileSelection.truncated')}
              </div>
            )}
          </div>
        )}
      </React.Fragment>
    );
  };

  const selectionHint = selectedPath
    ? `${t('fileSelection.selectedLocation')}: ${selectedPath}`
    : currentPath
      ? `${t('fileSelection.currentLocation')}: ${currentPath}`
      : isFileMode
        ? t('fileSelection.pleaseSelectFile')
        : t('fileSelection.pleaseSelectDirectory');

  return (
    <Modal
      visible={visible}
      title={
        <div className={styles.title}>
          <span className={styles.titleIcon}>
            {isFileMode ? <IconFile /> : <IconFolder />}
          </span>
          <span>{isFileMode ? t('fileSelection.selectFile') : t('fileSelection.selectDirectory')}</span>
        </div>
      }
      onCancel={onCancel}
      onOk={handleConfirm}
      okButtonProps={{ disabled: !selectedPath }}
      className={`nomifun-file-picker-modal ${styles.modal}`}
      style={{ width: 'min(700px, 92vw)' }}
      wrapStyle={{ zIndex: 3000 }}
      maskStyle={{ zIndex: 2990 }}
      alignCenter
      footer={
        <div className={styles.footer}>
          {!isFileMode && (
            <Button
              type='text'
              className={styles.newFolderButton}
              icon={<IconFolderAdd />}
              disabled={!canCreateFolder || creatingFolder}
              title={canCreateFolder ? t('fileSelection.newFolder') : t('fileSelection.selectParentForNewFolder')}
              onClick={() => void handleStartCreateFolder()}
            >
              {t('fileSelection.newFolder')}
            </Button>
          )}
          <div className={styles.footerActions}>
            <Button className={styles.cancelButton} onClick={onCancel}>
              {t('common.cancel')}
            </Button>
            <Button
              type='primary'
              className={`nomifun-file-picker-confirm ${styles.confirmButton}`}
              onClick={handleConfirm}
              disabled={!selectedPath}
            >
              {t('common.confirm')}
            </Button>
          </div>
        </div>
      }
    >
      <div className={styles.pickerBody}>
        <Spin loading={loading} className={styles.spin}>
          <div
            className={styles.treeViewport}
            role='tree'
            aria-label={isFileMode ? t('fileSelection.selectFile') : t('fileSelection.selectDirectory')}
          >
            {error ? (
              <div className={styles.initialError}>
                <span>{error}</span>
                <Button size='small' onClick={() => void loadInitialTree(currentPath)}>
                  {t('common.retry')}
                </Button>
              </div>
            ) : (
              treeNodes.map((node) => renderNode(node, 0))
            )}
          </div>
        </Spin>
        <div className={styles.selectionBar} title={selectionHint}>
          <span className={selectedPath ? styles.selectionDotActive : styles.selectionDot} />
          <span>{selectionHint}</span>
        </div>
      </div>
    </Modal>
  );
};

export default DirectorySelectionModal;
