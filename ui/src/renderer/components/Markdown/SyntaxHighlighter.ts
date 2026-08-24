/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

// Keep conversation highlighting on PrismLight. The package's Highlight.js
// adapter still couples lowlight 1.x to Highlight.js 10, while another UI
// dependency ships Highlight.js 11; a mis-resolved production bundle makes
// that old emitter crash on `startScope` during message rendering.
import SyntaxHighlighter from 'react-syntax-highlighter/dist/esm/prism-light';
import bash from 'react-syntax-highlighter/dist/esm/languages/prism/bash';
import c from 'react-syntax-highlighter/dist/esm/languages/prism/c';
import cpp from 'react-syntax-highlighter/dist/esm/languages/prism/cpp';
import csharp from 'react-syntax-highlighter/dist/esm/languages/prism/csharp';
import css from 'react-syntax-highlighter/dist/esm/languages/prism/css';
import diff from 'react-syntax-highlighter/dist/esm/languages/prism/diff';
import docker from 'react-syntax-highlighter/dist/esm/languages/prism/docker';
import go from 'react-syntax-highlighter/dist/esm/languages/prism/go';
import ini from 'react-syntax-highlighter/dist/esm/languages/prism/ini';
import java from 'react-syntax-highlighter/dist/esm/languages/prism/java';
import javascript from 'react-syntax-highlighter/dist/esm/languages/prism/javascript';
import json from 'react-syntax-highlighter/dist/esm/languages/prism/json';
import jsx from 'react-syntax-highlighter/dist/esm/languages/prism/jsx';
import kotlin from 'react-syntax-highlighter/dist/esm/languages/prism/kotlin';
import latex from 'react-syntax-highlighter/dist/esm/languages/prism/latex';
import lua from 'react-syntax-highlighter/dist/esm/languages/prism/lua';
import makefile from 'react-syntax-highlighter/dist/esm/languages/prism/makefile';
import markdown from 'react-syntax-highlighter/dist/esm/languages/prism/markdown';
import markup from 'react-syntax-highlighter/dist/esm/languages/prism/markup';
import php from 'react-syntax-highlighter/dist/esm/languages/prism/php';
import powershell from 'react-syntax-highlighter/dist/esm/languages/prism/powershell';
import python from 'react-syntax-highlighter/dist/esm/languages/prism/python';
import ruby from 'react-syntax-highlighter/dist/esm/languages/prism/ruby';
import rust from 'react-syntax-highlighter/dist/esm/languages/prism/rust';
import scss from 'react-syntax-highlighter/dist/esm/languages/prism/scss';
import sql from 'react-syntax-highlighter/dist/esm/languages/prism/sql';
import swift from 'react-syntax-highlighter/dist/esm/languages/prism/swift';
import tsx from 'react-syntax-highlighter/dist/esm/languages/prism/tsx';
import typescript from 'react-syntax-highlighter/dist/esm/languages/prism/typescript';
import vbnet from 'react-syntax-highlighter/dist/esm/languages/prism/vbnet';
import yaml from 'react-syntax-highlighter/dist/esm/languages/prism/yaml';

const languages = {
  bash,
  c,
  cpp,
  csharp,
  css,
  diff,
  docker,
  go,
  ini,
  java,
  javascript,
  json,
  jsx,
  kotlin,
  latex,
  lua,
  makefile,
  markdown,
  php,
  markup,
  powershell,
  python,
  ruby,
  rust,
  scss,
  sql,
  swift,
  tsx,
  typescript,
  vbnet,
  yaml,
};

for (const [name, grammar] of Object.entries(languages)) {
  SyntaxHighlighter.registerLanguage(name, grammar);
}

export { default as vs } from 'react-syntax-highlighter/dist/esm/styles/prism/vs';
export { default as vs2015 } from 'react-syntax-highlighter/dist/esm/styles/prism/vsc-dark-plus';
export default SyntaxHighlighter;
