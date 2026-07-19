/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

const URI_SCHEME_RE = /^[A-Za-z][A-Za-z\d+.-]*:/;
const WINDOWS_DRIVE_ABSOLUTE_RE = /^[A-Za-z]:[\\/]/;
const WINDOWS_EXTENDED_DRIVE_RE = /^(?:\\\\|\/\/)\?[\\/][A-Za-z]:[\\/]/;
const WINDOWS_EXTENDED_UNC_RE = /^(?:\\\\|\/\/)\?[\\/]UNC[\\/][^\\/]+[\\/][^\\/]+(?:[\\/]|$)/i;
const WINDOWS_UNC_RE = /^(?:\\\\|\/\/)(?![?.][\\/])[^\\/]+[\\/][^\\/]+(?:[\\/]|$)/;

export type ResolvedImageSource =
  | { kind: 'direct'; url: string }
  | { kind: 'local'; path: string; workspace?: string };

export const safeDecodeUriComponent = (value: string): string => {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
};

export const isFileUri = (value: string): boolean => /^file:/i.test(value);

export const isWindowsDriveAbsolutePath = (value: string): boolean => WINDOWS_DRIVE_ABSOLUTE_RE.test(value);

export const isWindowsUncPath = (value: string): boolean =>
  WINDOWS_UNC_RE.test(value) || WINDOWS_EXTENDED_UNC_RE.test(value);

export const isAbsoluteLocalPath = (value: string): boolean =>
  value.startsWith('/') ||
  isWindowsDriveAbsolutePath(value) ||
  isWindowsUncPath(value) ||
  WINDOWS_EXTENDED_DRIVE_RE.test(value);

/** Convert a standards-based file URI into the OS path expected by the backend. */
export const fileUriToPath = (value: string): string | null => {
  if (!isFileUri(value)) return null;

  try {
    const url = new URL(value);
    if (url.protocol.toLowerCase() !== 'file:') return null;

    const pathname = safeDecodeUriComponent(url.pathname);
    const hostname = safeDecodeUriComponent(url.hostname);

    // A non-local host denotes a Windows UNC share. URL.pathname deliberately
    // excludes the authority, so put it back before passing the path to Rust.
    if (hostname && hostname.toLowerCase() !== 'localhost') {
      return `//${hostname}${pathname.startsWith('/') ? pathname : `/${pathname}`}`;
    }

    // WHATWG file URLs spell Windows drive paths as /C:/..., while Path on
    // Windows expects C:/... (or C:\\...).
    if (/^\/[A-Za-z]:[\\/]/.test(pathname)) {
      return pathname.slice(1);
    }

    return pathname;
  } catch {
    return null;
  }
};

/** True for sources the browser should load directly instead of the local-FS API. */
export const isDirectImageSource = (value: string): boolean => {
  if (!value || isFileUri(value) || isAbsoluteLocalPath(value) || /^[A-Za-z]:/.test(value)) return false;
  return URI_SCHEME_RE.test(value);
};

/** Relative paths, native absolute paths, UNC paths, and file URIs are local. */
export const isLocalImageSource = (value: string): boolean => Boolean(value) && !isDirectImageSource(value);

type ParsedPath = {
  prefix: string;
  parts: string[];
  separator: '/' | '\\';
  prefixHasTrailingSeparator: boolean;
};

const splitParts = (value: string | undefined): string[] => (value ? value.split(/[\\/]+/).filter(Boolean) : []);

const parseBasePath = (value: string): ParsedPath => {
  const separator: '/' | '\\' = value.includes('\\') && !value.includes('/') ? '\\' : '/';

  const extendedUnc = value.match(/^(?:\\\\|\/\/)\?[\\/]UNC[\\/]([^\\/]+)[\\/]([^\\/]+)(?:[\\/]+(.*))?$/i);
  if (extendedUnc) {
    return {
      prefix: `${separator}${separator}?${separator}UNC${separator}${extendedUnc[1]}${separator}${extendedUnc[2]}`,
      parts: splitParts(extendedUnc[3]),
      separator,
      prefixHasTrailingSeparator: false,
    };
  }

  const extendedDrive = value.match(/^(?:\\\\|\/\/)\?[\\/]([A-Za-z]:)[\\/]*(.*)$/);
  if (extendedDrive) {
    return {
      prefix: `${separator}${separator}?${separator}${extendedDrive[1]}${separator}`,
      parts: splitParts(extendedDrive[2]),
      separator,
      prefixHasTrailingSeparator: true,
    };
  }

  const unc = value.match(/^(?:\\\\|\/\/)([^\\/]+)[\\/]([^\\/]+)(?:[\\/]+(.*))?$/);
  if (unc) {
    return {
      prefix: `${separator}${separator}${unc[1]}${separator}${unc[2]}`,
      parts: splitParts(unc[3]),
      separator,
      prefixHasTrailingSeparator: false,
    };
  }

  const drive = value.match(/^([A-Za-z]:)[\\/]*(.*)$/);
  if (drive) {
    return {
      prefix: `${drive[1]}${separator}`,
      parts: splitParts(drive[2]),
      separator,
      prefixHasTrailingSeparator: true,
    };
  }

  if (value.startsWith('/')) {
    return {
      prefix: '/',
      parts: splitParts(value),
      separator: '/',
      prefixHasTrailingSeparator: true,
    };
  }

  return { prefix: '', parts: splitParts(value), separator, prefixHasTrailingSeparator: false };
};

const appendRelativeParts = (parts: string[], relativePath: string): string[] => {
  const result = [...parts];
  for (const part of relativePath.split(/[\\/]+/)) {
    if (!part || part === '.') continue;
    if (part === '..') {
      result.pop();
    } else {
      result.push(part);
    }
  }
  return result;
};

const isWorkspaceContainedRelativePath = (value: string): boolean => {
  // `C:foo` is drive-relative on Windows and resolves against process state,
  // not the supplied workspace. Never pass it to the local-file endpoint.
  if (/^[A-Za-z]:/.test(value)) return false;

  let depth = 0;
  for (const part of value.split(/[\\/]+/)) {
    if (!part || part === '.') continue;
    if (part === '..') {
      if (depth === 0) return false;
      depth -= 1;
    } else {
      depth += 1;
    }
  }
  return true;
};

/**
 * Join a directory path without flattening URI (`scheme://`) or UNC (`//host`)
 * prefixes. A filesystem path keeps the separator style of its base path.
 */
export const joinLocalPath = (basePath: string, relativePath: string): string => {
  if (!basePath) return relativePath;
  if (!relativePath) return basePath;

  // An absolute/URI second argument is already fully resolved.
  if (isFileUri(relativePath) || isAbsoluteLocalPath(relativePath) || URI_SCHEME_RE.test(relativePath)) {
    return relativePath;
  }

  // URL bases need URL semantics for query/fragment encoding and parent dirs.
  // Native drive paths were excluded above before testing the generic scheme.
  if (URI_SCHEME_RE.test(basePath) && !isWindowsDriveAbsolutePath(basePath)) {
    try {
      const directoryBase = basePath.endsWith('/') ? basePath : `${basePath}/`;
      return new URL(relativePath.replace(/\\/g, '/'), directoryBase).toString();
    } catch {
      return relativePath;
    }
  }

  const parsed = parseBasePath(basePath);
  const parts = appendRelativeParts(parsed.parts, relativePath);
  const suffix = parts.join(parsed.separator);
  if (!parsed.prefix) return suffix;
  if (!suffix) return parsed.prefix;
  return parsed.prefixHasTrailingSeparator ? `${parsed.prefix}${suffix}` : `${parsed.prefix}${parsed.separator}${suffix}`;
};

/** Resolve an image source into either a browser URL or a backend-readable path. */
export const resolveImageSource = (src: string, root = ''): ResolvedImageSource => {
  if (isDirectImageSource(src)) {
    return { kind: 'direct', url: src };
  }

  const filePath = fileUriToPath(src);
  const decodedPath = filePath ?? safeDecodeUriComponent(src);
  const workspace = (fileUriToPath(root) ?? root) || undefined;

  if (!workspace || filePath !== null || isAbsoluteLocalPath(decodedPath)) {
    return { kind: 'local', path: decodedPath, workspace };
  }

  if (!isWorkspaceContainedRelativePath(decodedPath)) {
    return { kind: 'local', path: '', workspace };
  }

  return { kind: 'local', path: joinLocalPath(workspace, decodedPath), workspace };
};
