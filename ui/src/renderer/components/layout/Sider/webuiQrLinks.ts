/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

type WebuiQrStatusLike = {
  allowRemote?: boolean;
  localUrl?: string;
  networkUrl?: string;
  networkUrls?: string[];
  port?: number;
};

const normalizeBaseUrl = (url: string): string => url.trim().replace(/\/+$/, '');

const parseIpv4Host = (host: string): number[] | null => {
  const parts = host.split('.');
  if (parts.length !== 4) return null;
  const octets = parts.map((part) => {
    if (!/^\d+$/.test(part)) return Number.NaN;
    return Number(part);
  });
  return octets.every((octet) => Number.isInteger(octet) && octet >= 0 && octet <= 255) ? octets : null;
};

const isUsableRemoteHost = (host: string): boolean => {
  const normalized = host.trim().toLowerCase();
  if (!normalized || normalized === 'localhost') return false;

  const ipv4 = parseIpv4Host(normalized);
  if (!ipv4) return true;

  const [a, b, c] = ipv4;
  if (a === 0 || a === 127) return false;
  if (a === 169 && b === 254) return false;
  if (a === 198 && (b === 18 || b === 19)) return false;
  if (a === 192 && b === 0 && c === 2) return false;
  if (a === 198 && b === 51 && c === 100) return false;
  if (a === 203 && b === 0 && c === 113) return false;
  if (a >= 224) return false;
  return true;
};

const isUsableRemoteUrl = (url: string): boolean => {
  try {
    return isUsableRemoteHost(new URL(url).hostname);
  } catch {
    return true;
  }
};

const pushUniqueUrl = (urls: string[], url?: string | null, remote = false) => {
  if (!url) return;
  const normalized = normalizeBaseUrl(url);
  if (remote && !isUsableRemoteUrl(normalized)) return;
  if (normalized && !urls.includes(normalized)) {
    urls.push(normalized);
  }
};

export const getWebuiQrBaseUrls = (
  status: WebuiQrStatusLike | null | undefined,
  accessUrls: string[],
  fallbackPort: number
): string[] => {
  const remoteUrls: string[] = [];
  for (const url of accessUrls) pushUniqueUrl(remoteUrls, url, true);
  for (const url of status?.networkUrls ?? []) pushUniqueUrl(remoteUrls, url, true);
  pushUniqueUrl(remoteUrls, status?.networkUrl, true);

  if (status?.allowRemote && remoteUrls.length > 0) {
    return remoteUrls;
  }

  const localUrls: string[] = [];
  pushUniqueUrl(localUrls, status?.localUrl);
  if (localUrls.length > 0) return localUrls;
  return [`http://localhost:${status?.port ?? fallbackPort}`];
};

export const buildWebuiQrLoginUrl = (baseUrl: string, token: string): string =>
  `${normalizeBaseUrl(baseUrl)}/qr-login?token=${encodeURIComponent(token)}`;
