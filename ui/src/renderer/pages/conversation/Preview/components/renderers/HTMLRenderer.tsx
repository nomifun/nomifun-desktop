/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { useTypingAnimation } from '@/renderer/hooks/chat/useTypingAnimation';
import React, { useEffect, useMemo, useRef, useState } from 'react';

interface HTMLRendererProps {
  content: string;
  file_path?: string;
  workspace?: string;
  containerRef?: React.RefObject<HTMLDivElement | null>;
  onScroll?: (scrollTop: number, scrollHeight: number, clientHeight: number) => void;
}

/**
 * 解析相对路径为绝对路径 / Resolve relative path to absolute path
 * @param basePath 基础文件路径 / Base file path
 * @param relativePath 相对路径 / Relative path
 * @returns 绝对路径 / Absolute path
 */
function resolveRelativePath(basePath: string, relativePath: string): string {
  // 去除协议前缀 / Remove protocol prefix
  const cleanBasePath = basePath.replace(/^file:\/\//, '');
  const baseDir =
    cleanBasePath.substring(0, cleanBasePath.lastIndexOf('/') + 1) ||
    cleanBasePath.substring(0, cleanBasePath.lastIndexOf('\\') + 1);

  // 如果相对路径已经是绝对路径，直接返回 / If relative path is already absolute, return directly
  if (relativePath.startsWith('/') || /^[a-zA-Z]:/.test(relativePath)) {
    return relativePath;
  }

  // 处理 ./ 和 ../ / Handle ./ and ../
  const parts = baseDir.replace(/\\/g, '/').split('/').filter(Boolean);
  const relParts = relativePath.replace(/\\/g, '/').split('/');

  for (const part of relParts) {
    if (part === '..') {
      parts.pop();
    } else if (part !== '.') {
      parts.push(part);
    }
  }

  // 保留 Windows 盘符格式 / Preserve Windows drive letter format
  if (/^[a-zA-Z]:/.test(baseDir)) {
    return parts.join('/');
  }
  return '/' + parts.join('/');
}

/**
 * 内联化 HTML 中的相对资源（用于 browser iframe）
 * Inline relative resources in HTML (for browser iframe)
 *
 * - img src -> base64 data URL
 * - link href (CSS) -> inline <style> tag
 * - script src -> inline <script> tag
 *
 * @param html HTML 内容 / HTML content
 * @param basePath 基础文件路径 / Base file path
 * @returns 处理后的 HTML / Processed HTML
 */
async function inlineRelativeResources(html: string, basePath: string, workspace?: string): Promise<string> {
  let result = html;

  // 1. 处理 <img src="relative"> -> base64 / Handle <img src="relative"> -> base64
  const imgRegex = /<img([^>]*)\ssrc=["'](?!https?:\/\/|data:|\/\/)([^"']+)["']([^>]*)>/gi;
  const imgMatches = [...result.matchAll(imgRegex)];

  for (const match of imgMatches) {
    const [fullMatch, before, src, after] = match;
    try {
      const absolutePath = resolveRelativePath(basePath, src);
      const dataUrl = await ipcBridge.fs.getImageBase64.invoke({ path: absolutePath, workspace });
      if (dataUrl) {
        // getImageBase64 已经返回完整的 data URL / getImageBase64 already returns complete data URL
        const newTag = `<img${before} src="${dataUrl}"${after}>`;
        result = result.replace(fullMatch, newTag);
      }
    } catch (e) {
      console.warn('[HTMLRenderer] Failed to inline image:', src, e);
    }
  }

  // 2. 处理 <link href="relative" rel="stylesheet"> -> <style> / Handle CSS links -> inline <style>
  const linkRegex = /<link([^>]*)\shref=["'](?!https?:\/\/|data:|\/\/)([^"']+)["']([^>]*)>/gi;
  const linkMatches = [...result.matchAll(linkRegex)];

  for (const match of linkMatches) {
    const [fullMatch, _before, href, _after] = match;
    // 检查是否为 stylesheet / Check if it's a stylesheet
    const isStylesheet = /rel=["']stylesheet["']/i.test(fullMatch) || href.endsWith('.css');
    if (isStylesheet) {
      try {
        const absolutePath = resolveRelativePath(basePath, href);
        const cssContent = await ipcBridge.fs.readFile.invoke({ path: absolutePath, workspace });
        if (cssContent) {
          // 替换 CSS 中的相对 url() 引用为 base64 / Replace relative url() references in CSS with base64
          let processedCss = cssContent;
          const cssUrlRegex = /url\(["']?(?!https?:\/\/|data:|\/\/)([^"')]+)["']?\)/gi;
          const cssUrlMatches = [...processedCss.matchAll(cssUrlRegex)];

          for (const urlMatch of cssUrlMatches) {
            const [urlFullMatch, urlPath] = urlMatch;
            try {
              // CSS 文件的基础路径 / Base path for CSS file
              const cssBasePath = absolutePath;
              const resourcePath = resolveRelativePath(cssBasePath, urlPath);
              const dataUrl = await ipcBridge.fs.getImageBase64.invoke({ path: resourcePath, workspace });
              if (dataUrl) {
                // getImageBase64 已经返回完整的 data URL / getImageBase64 already returns complete data URL
                processedCss = processedCss.replace(urlFullMatch, `url("${dataUrl}")`);
              }
            } catch (e) {
              console.warn('[HTMLRenderer] Failed to inline CSS resource:', urlPath, e);
            }
          }

          const styleTag = `<style>${processedCss}</style>`;
          result = result.replace(fullMatch, styleTag);
        }
      } catch (e) {
        console.warn('[HTMLRenderer] Failed to inline CSS:', href, e);
      }
    }
  }

  // 3. 处理 <script src="relative"> -> inline <script> / Handle script tags -> inline
  const scriptRegex = /<script([^>]*)\ssrc=["'](?!https?:\/\/|data:|\/\/)([^"']+)["']([^>]*)><\/script>/gi;
  const scriptMatches = [...result.matchAll(scriptRegex)];

  for (const match of scriptMatches) {
    const [fullMatch, before, src, after] = match;
    try {
      const absolutePath = resolveRelativePath(basePath, src);
      const scriptContent = await ipcBridge.fs.readFile.invoke({ path: absolutePath, workspace });
      if (scriptContent) {
        // 保留其他属性（如 type, defer, async 等，但 async/defer 对 inline 无效）
        // Keep other attributes (like type, but defer/async don't work for inline)
        const attrsToKeep = (before + after).replace(/\s*(defer|async)\s*/gi, '');
        const scriptTag = `<script${attrsToKeep}>${scriptContent}</script>`;
        result = result.replace(fullMatch, scriptTag);
      }
    } catch (e) {
      console.warn('[HTMLRenderer] Failed to inline script:', src, e);
    }
  }

  return result;
}

/**
 * HTML 渲染器组件
 * HTML renderer component
 *
 * 在 iframe 中渲染 HTML 内容，相对资源通过 ipcBridge.fs.* 内联化
 * Renders HTML content in an iframe; relative resources are inlined via ipcBridge.fs.*
 */
const HTMLRenderer: React.FC<HTMLRendererProps> = ({ content, file_path, workspace, containerRef, onScroll }) => {
  const divRef = useRef<HTMLDivElement>(null);
  const iframeRef = useRef<HTMLIFrameElement | null>(null);
  const [inlinedHtmlContent, setInlinedHtmlContent] = useState<string>(''); // 内联化后的 HTML / Inlined HTML
  const [currentTheme, setCurrentTheme] = useState<'light' | 'dark'>(() => {
    return (document.documentElement.getAttribute('data-theme') as 'light' | 'dark') || 'light';
  });

  // 监听主题变化 / Monitor theme changes
  useEffect(() => {
    const updateTheme = () => {
      const theme = (document.documentElement.getAttribute('data-theme') as 'light' | 'dark') || 'light';
      setCurrentTheme(theme);
    };

    const observer = new MutationObserver(updateTheme);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-theme'],
    });

    return () => observer.disconnect();
  }, []);

  // 检查是否有相对资源（用于 inline 处理）
  // Check if has relative resources (for inline processing)
  const hasRelativeResources = useMemo(() => {
    return (
      /<link[^>]+href=["'](?!https?:\/\/|data:|\/\/)[^"']+["']/i.test(content) ||
      /<script[^>]+src=["'](?!https?:\/\/|data:|\/\/)[^"']+["']/i.test(content) ||
      /<img[^>]+src=["'](?!https?:\/\/|data:|\/\/)[^"']+["']/i.test(content)
    );
  }, [content]);

  // 流式打字动画：HTML 预览在使用 data URL 渲染时也能获得流式体验
  // Typing animation: provide streaming experience when rendering via data URL
  const { displayedContent } = useTypingAnimation({
    content,
    enabled: !hasRelativeResources,
    speed: 40,
  });

  // 当存在相对资源时进行内联化处理
  // Inline relative resources when present
  useEffect(() => {
    if (!hasRelativeResources || !file_path) {
      // 没有相对资源或没有文件路径，使用原始内容
      // No relative resources or no file path, use original content
      setInlinedHtmlContent(content);
      return;
    }

    let cancelled = false;
    inlineRelativeResources(content, file_path, workspace)
      .then((inlined) => {
        if (!cancelled) {
          setInlinedHtmlContent(inlined);
        }
      })
      .catch((e) => {
        console.warn('[HTMLRenderer] Failed to inline resources:', e);
        if (!cancelled) {
          setInlinedHtmlContent(content); // 回退到原始内容 / Fallback to original content
        }
      });

    return () => {
      cancelled = true;
    };
  }, [content, file_path, hasRelativeResources, workspace]);

  // 用于 iframe 的最终 HTML 内容
  // Final HTML content for iframe
  const browserHtmlContent = useMemo(() => {
    if (hasRelativeResources && file_path) {
      return inlinedHtmlContent || content; // 在内联化完成前显示原始内容 / Show original content before inlining completes
    }
    return displayedContent;
  }, [hasRelativeResources, file_path, inlinedHtmlContent, content, displayedContent]);

  // preview→editor 滚动同步：iframe 内部文档才是实际滚动容器，监听其滚动并上报
  // preview→editor scroll sync: the iframe inner document is the real scroll container
  useEffect(() => {
    const iframe = iframeRef.current;
    if (!iframe || !onScroll) return;

    let detach: (() => void) | undefined;
    const attach = () => {
      detach?.();
      detach = undefined;
      try {
        const win = iframe.contentWindow;
        const doc = win?.document;
        if (!win || !doc) return;
        const handler = () => {
          const el = doc.scrollingElement || doc.documentElement;
          if (el) onScroll(el.scrollTop, el.scrollHeight, el.clientHeight);
        };
        win.addEventListener('scroll', handler, { passive: true });
        detach = () => win.removeEventListener('scroll', handler);
      } catch {
        // 内容不可访问时静默跳过（如被导航到跨域页面）/ Silently skip when inner document is inaccessible
      }
    };

    attach(); // srcDoc 可能已加载完成 / srcDoc may already be loaded
    iframe.addEventListener('load', attach);
    return () => {
      iframe.removeEventListener('load', attach);
      detach?.();
    };
  }, [onScroll, browserHtmlContent]);

  return (
    <div
      ref={containerRef || divRef}
      className={`h-full w-full overflow-auto relative ${currentTheme === 'dark' ? 'bg-1' : 'bg-white'}`}
    >
      <iframe
        ref={iframeRef}
        srcDoc={browserHtmlContent}
        className='w-full h-full border-0'
        style={{
          display: 'block',
          width: '100%',
          height: '100%',
        }}
        sandbox='allow-scripts allow-same-origin allow-forms allow-popups allow-modals'
      />
    </div>
  );
};

export default HTMLRenderer;
