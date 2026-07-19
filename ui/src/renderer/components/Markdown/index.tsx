/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import ReactMarkdown, { defaultUrlTransform } from 'react-markdown';
import type { Components, ExtraProps, UrlTransform } from 'react-markdown';

import rehypeKatex from 'rehype-katex';
import rehypeRaw from 'rehype-raw';
import remarkBreaks from 'remark-breaks';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';

// Import KaTeX CSS to make it available in the document
import 'katex/dist/katex.min.css';

import { openExternalUrl } from '@/renderer/utils/platform';
import classNames from 'classnames';
import React, { useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { convertLatexDelimiters } from '@renderer/utils/chat/latexDelimiters';
import LocalImageView from '@renderer/components/media/LocalImageView';
import { isLocalImageSource } from '@/common/utils/localPath';
import CodeBlock from './CodeBlock';
import ShadowView from './ShadowView';

const REMARK_PLUGINS = [remarkGfm, remarkMath, remarkBreaks];

const markdownUrlTransform: UrlTransform = (url, key, node) => {
  if (key === 'src' && node.tagName === 'img') {
    // react-markdown rejects unknown schemes by default. Permit filesystem
    // references for LocalImageView and inline/blob images for the browser,
    // while leaving every other protocol on the library's safe allowlist.
    if (isLocalImageSource(url) || /^data:image\//i.test(url) || /^blob:/i.test(url)) {
      return url;
    }
  }
  return defaultUrlTransform(url);
};

type MarkdownViewProps = {
  children: string;
  hiddenCodeCopyButton?: boolean;
  codeStyle?: React.CSSProperties;
  className?: string;
  onRef?: (el?: HTMLDivElement | null) => void;
  fontSize?: string;
  lineHeight?: string;
  /** Enable raw HTML rendering in markdown content. Use with caution — only for trusted sources. */
  allowHtml?: boolean;
  /** Model/tool Markdown is not a verified artifact-delivery receipt. */
  allowUnverifiedImages?: boolean;
};

const MarkdownView: React.FC<MarkdownViewProps> = React.memo(
  ({
    hiddenCodeCopyButton,
    codeStyle,
    className,
    onRef,
    fontSize,
    lineHeight,
    allowHtml,
    allowUnverifiedImages = true,
    children: childrenProp,
  }) => {
    const { t } = useTranslation();

    const normalizedChildren = useMemo(() => {
      if (typeof childrenProp === 'string') {
        return convertLatexDelimiters(childrenProp);
      }
      return childrenProp;
    }, [childrenProp]);

    const handleLinkClick = useCallback(
      (e: React.MouseEvent<HTMLAnchorElement>) => {
        e.preventDefault();
        e.stopPropagation();
        const href = (e.currentTarget as HTMLAnchorElement).href;
        if (!href) return;
        openExternalUrl(href).catch((error: unknown) => {
          console.error(t('messages.openLinkFailed'), error);
        });
      },
      [t]
    );

    // Memoize components so React preserves component identity across re-renders.
    // Without this, every streaming update creates new function references → React
    // unmounts/remounts all custom components → hooks & DOM state are lost.
    const components = useMemo<Components>(
      () => ({
        span: ({ node: _node, className: cn, children: ch, ...rest }: React.JSX.IntrinsicElements['span'] & ExtraProps) => (
          <span {...rest} className={cn}>
            {ch}
          </span>
        ),
        code: (props: React.JSX.IntrinsicElements['code'] & ExtraProps) => (
          <CodeBlock
            {...(props as Parameters<typeof CodeBlock>[0])}
            codeStyle={codeStyle}
            hiddenCodeCopyButton={hiddenCodeCopyButton}
          />
        ),
        a: ({ node: _node, ...rest }: React.JSX.IntrinsicElements['a'] & ExtraProps) => (
          <a {...rest} target='_blank' rel='noreferrer' onClick={handleLinkClick} />
        ),
        table: ({ node: _node, style, ...rest }: React.JSX.IntrinsicElements['table'] & ExtraProps) => (
          <div style={{ overflowX: 'auto', maxWidth: '100%' }}>
            <table
              {...rest}
              style={{
                ...style,
                borderCollapse: 'collapse',
                border: '1px solid var(--bg-3)',
                minWidth: '100%',
              }}
            />
          </div>
        ),
        td: ({ node: _node, style, ...rest }: React.JSX.IntrinsicElements['td'] & ExtraProps) => (
          <td
            {...rest}
            style={{
              ...style,
              padding: '8px',
              border: '1px solid var(--bg-3)',
              minWidth: '120px',
            }}
          />
        ),
        img: ({ node: _node, ...rest }: React.JSX.IntrinsicElements['img'] & ExtraProps) => {
          const imgProps = rest;
          const src = imgProps.src || '';
          if (!allowUnverifiedImages) {
            return (
              <span className={imgProps.className}>
                Unverified image reference: {imgProps.alt || src || 'unknown source'}
              </span>
            );
          }
          if (isLocalImageSource(src)) {
            return <LocalImageView src={src} alt={imgProps.alt || ''} className={imgProps.className} />;
          }
          return <img {...imgProps} />;
        },
      }),
      [allowUnverifiedImages, codeStyle, hiddenCodeCopyButton, handleLinkClick]
    );

    const rehypePlugins = useMemo(() => (allowHtml ? [rehypeRaw, rehypeKatex] : [rehypeKatex]), [allowHtml]);

    return (
      <div className={classNames('relative w-full', className)}>
        <ShadowView fontSize={fontSize} lineHeight={lineHeight}>
          <div ref={onRef} className='markdown-shadow-body'>
            <ReactMarkdown
              remarkPlugins={REMARK_PLUGINS}
              rehypePlugins={rehypePlugins}
              components={components}
              urlTransform={markdownUrlTransform}
            >
              {normalizedChildren}
            </ReactMarkdown>
          </div>
        </ShadowView>
      </div>
    );
  }
);

MarkdownView.displayName = 'MarkdownView';

export default MarkdownView;
