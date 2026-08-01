/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * NomiRouter 协议检测类型
 * Protocol detection types for NomiRouter
 *
 * 实际检测逻辑在后端：POST /api/providers/detect-protocol
 * （crates/backend/nomifun-system/src/protocol.rs），本文件只保留 wire 类型。
 * Detection runtime lives in the backend (POST /api/providers/detect-protocol,
 * crates/backend/nomifun-system/src/protocol.rs); this module keeps only the wire types.
 */

/**
 * 支持的协议类型
 * Supported protocol types
 */
export type ProtocolType = 'openai' | 'gemini' | 'anthropic' | 'unknown';

/**
 * 多 Key 测试结果
 * Multi-key test result
 */
export interface MultiKeyTestResult {
  /** 总 Key 数量 / Total key count */
  total: number;
  /** 有效 Key 数量 / Valid key count */
  valid: number;
  /** 无效 Key 数量 / Invalid key count */
  invalid: number;
  /** 每个 Key 的详细结果 / Detailed result for each key */
  details: Array<{
    /** Key 索引 / Key index */
    index: number;
    /** Key 掩码（只显示前后几位）/ Masked key */
    maskedKey: string;
    /** 是否有效 / Whether valid */
    valid: boolean;
    /** 错误信息 / Error message */
    error?: string;
    /** 响应时间 / Latency */
    latency?: number;
  }>;
}

/**
 * 协议检测请求参数
 * Protocol detection request parameters
 */
export interface ProtocolDetectionRequest {
  /** Base URL */
  base_url: string;
  /** API Key（多个 Key 使用英文逗号分隔）/ API Key (comma-separated for multiple keys) */
  api_key: string;
  /** 超时时间（毫秒）/ Timeout in milliseconds */
  timeout?: number;
  /** 是否测试所有 Key（默认只测试第一个）/ Whether to test all keys */
  testAllKeys?: boolean;
  /** 指定要测试的协议（如果已知）/ Specific protocol to test (if known) */
  preferredProtocol?: ProtocolType;
}

/**
 * 协议检测响应
 * Protocol detection response
 */
export interface ProtocolDetectionResponse {
  /** 是否成功 / Whether successful */
  success: boolean;
  /** 检测到的协议 / Detected protocol */
  protocol: ProtocolType;
  /** 置信度 / Confidence */
  confidence: number;
  /** 错误信息 / Error message */
  error?: string;
  /** 修正后的 base URL / Fixed base URL */
  fixedBaseUrl?: string;
  /** 建议操作 / Suggested action */
  suggestion?: {
    /** 建议类型 / Suggestion type */
    type: 'switch_platform' | 'fix_url' | 'check_key' | 'none';
    /** 建议消息 / Suggestion message */
    message: string;
    /** 建议的平台 / Suggested platform */
    suggestedPlatform?: string;
    /** i18n key（前端使用）/ i18n key for frontend */
    i18nKey?: string;
    /** i18n 参数 / i18n parameters */
    i18nParams?: Record<string, string>;
  };
  /** 多 Key 测试结果（如果启用）/ Multi-key test result if enabled */
  multiKeyResult?: MultiKeyTestResult;
  /** 模型列表 / Model list */
  models?: string[];
  /**
   * 检测到的所有协议（聚合网关可能在同一地址同时提供多种协议，如 gpt + claude）
   * All protocols that succeeded — aggregator gateways may serve several on one base_url (e.g. gpt + claude).
   */
  detectedProtocols?: Array<{
    protocol: ProtocolType;
    confidence: number;
    models?: string[];
  }>;
}
