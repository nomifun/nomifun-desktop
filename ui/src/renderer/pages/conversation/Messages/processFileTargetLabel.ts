/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type FileTargetLabelOptions = {
  workspaceRoots?: Array<string | null | undefined>;
};

export type FileTargetLabel = {
  label: string;
  title: string;
  isWorkspaceTarget: boolean;
};

const toComparablePath = (value: string): string => {
  const trimmed = value.trim();
  if (!trimmed) return '';

  if (/^file:\/\//i.test(trimmed)) {
    try {
      return decodeURIComponent(new URL(trimmed).pathname).replace(/\\/g, '/').replace(/\/+$/, '');
    } catch {
      // Fall through to plain path normalization.
    }
  }

  return trimmed.replace(/\\/g, '/').replace(/\/+$/, '');
};

const isAbsolutePath = (value: string): boolean => {
  if (/^file:\/\//i.test(value)) return true;
  const comparable = toComparablePath(value);
  return comparable.startsWith('/') || /^[A-Za-z]:\//.test(comparable);
};

const getPathBasename = (value: string): string => {
  const comparable = toComparablePath(value);
  if (!comparable) return value.trim();
  const parts = comparable.split('/').filter(Boolean);
  return parts.at(-1) ?? value.trim();
};

const isInsideWorkspace = (target: string, workspaceRoots: Array<string | null | undefined> = []): boolean => {
  const comparableTarget = toComparablePath(target);
  if (!comparableTarget) return false;

  return workspaceRoots.some((root) => {
    if (!root) return false;
    const comparableRoot = toComparablePath(root);
    return Boolean(comparableRoot) && (comparableTarget === comparableRoot || comparableTarget.startsWith(`${comparableRoot}/`));
  });
};

export const splitToolReceiptTargets = (target?: string): string[] =>
  target
    ?.split(', ')
    .map((value) => value.trim())
    .filter(Boolean) ?? [];

export const formatWorkspaceFileTarget = (
  target: string,
  options: FileTargetLabelOptions = {}
): FileTargetLabel => {
  const title = target.trim();
  const isWorkspaceTarget = isInsideWorkspace(title, options.workspaceRoots);
  const shouldShorten = isWorkspaceTarget || !isAbsolutePath(title);
  const label = shouldShorten ? getPathBasename(title) : title;

  return {
    label: label || title,
    title,
    isWorkspaceTarget,
  };
};

export const formatFileTargetPreview = (targets: string[], options: FileTargetLabelOptions = {}): string =>
  targets.map((target) => formatWorkspaceFileTarget(target, options).label).join(', ');
