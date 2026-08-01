/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Google API 主机白名单
 * Google API Hosts Whitelist
 */
const GOOGLE_API_HOSTS: readonly string[] = [
  /** Gemini API */
  'generativelanguage.googleapis.com',
  /** Vertex AI */
  'aiplatform.googleapis.com',
];

/**
 * 安全验证 URL 是否为指定提供商的官方主机
 * Safely validate if URL is an official host for specified provider
 */
function isOfficialHost(urlString: string, allowedHosts: readonly string[]): boolean {
  try {
    const url = new URL(urlString);
    return allowedHosts.includes(url.hostname);
  } catch {
    return false;
  }
}

/**
 * 安全验证 URL 是否为 Google APIs 主机
 * Safely validate if URL is a Google APIs host
 *
 * 使用 URL 解析而非字符串包含检查，防止恶意 URL 绕过
 * Uses URL parsing instead of string includes to prevent malicious URL bypass
 *
 * @param urlString - 要验证的 URL 字符串 / URL string to validate
 * @returns 如果是有效的 Google APIs 主机返回 true / Returns true if valid Google APIs host
 *
 * @example
 * isGoogleApisHost('https://generativelanguage.googleapis.com/v1') // true
 * isGoogleApisHost('https://evil.com/generativelanguage.googleapis.com') // false
 * isGoogleApisHost('https://generativelanguage.googleapis.com.evil.com') // false
 */
export function isGoogleApisHost(urlString: string): boolean {
  return isOfficialHost(urlString, GOOGLE_API_HOSTS);
}
