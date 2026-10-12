import { createRequire } from 'node:module';

const require = createRequire(new URL('../ui/package.json', import.meta.url));
const ts = require('typescript');

const clientTerms = /\bmobile\b|手机|移动端|\bi(?:Phone|Pad|Pod|OS)\b|\bAndroid\b/g;
const connectionSources = /^ui\/src\/renderer\/(?:pages\/(?:connections|openCapabilities)\/|components\/layout\/Sider\/RelayPairingPanel(?:\.|\/))/;
const settingsLocale = /^ui\/src\/renderer\/services\/i18n\/locales\/[^/]+\/settings\.json$/;

function collectStringValues(node, jsonSource, ranges) {
  if (ts.isPropertyAssignment(node)) collectStringValues(node.initializer, jsonSource, ranges);
  else if (ts.isObjectLiteralExpression(node)) for (const property of node.properties) collectStringValues(property, jsonSource, ranges);
  else if (ts.isArrayLiteralExpression(node)) for (const element of node.elements) collectStringValues(element, jsonSource, ranges);
  else if (node.kind === ts.SyntaxKind.StringLiteral || node.kind === ts.SyntaxKind.NoSubstitutionTemplateLiteral) {
    ranges.push([node.getStart(jsonSource), node.getEnd()]);
  }
}

// The device-connection hub copy legitimately names the separate phone client;
// only that namespace inside settings.json is masked, so every other locale
// string stays checked.
function maskDeviceConnections(source) {
  const jsonSource = ts.parseJsonText('settings.json', source);
  const root = jsonSource.statements[0]?.expression;
  if (!root || !ts.isObjectLiteralExpression(root)) return source;
  const target = root.properties.find((property) =>
    ts.isPropertyAssignment(property) && property.name &&
    (ts.isStringLiteral(property.name) || ts.isIdentifier(property.name)) &&
    property.name.text === 'deviceConnections');
  if (!target) return source;
  const ranges = [];
  collectStringValues(target.initializer, jsonSource, ranges);
  let masked = source;
  for (const [start, end] of ranges.reverse()) {
    masked = masked.slice(0, start) + source.slice(start, end).replace(/[^\r\n]/g, ' ') + masked.slice(end);
  }
  return masked;
}

// Desktop connection management describes separate clients. Mask only literal
// text and comments in that bounded surface; executable layout decisions and
// all viewport/safe-area checks still inspect the original source.
function terminologySource(file, source) {
  if (settingsLocale.test(file)) return maskDeviceConnections(source);
  if (!connectionSources.test(file) || !/\.tsx?$/.test(file)) return source;
  const scanner = ts.createScanner(ts.ScriptTarget.Latest, false, ts.LanguageVariant.JSX, source);
  const ranges = [];
  for (let token = scanner.scan(); token !== ts.SyntaxKind.EndOfFileToken; token = scanner.scan()) {
    if ([ts.SyntaxKind.StringLiteral, ts.SyntaxKind.NoSubstitutionTemplateLiteral,
      ts.SyntaxKind.SingleLineCommentTrivia, ts.SyntaxKind.MultiLineCommentTrivia].includes(token)) {
      ranges.push([scanner.getTokenPos(), scanner.getTextPos()]);
    }
  }
  let masked = source;
  for (const [start, end] of ranges.reverse()) {
    masked = masked.slice(0, start) + source.slice(start, end).replace(/[^\r\n]/g, ' ') + masked.slice(end);
  }
  return masked;
}

export function inspectRendererSource(file, source, minimumWidth = 880) {
  const failures = [];
  const patterns = [
    [/\b(?:isMobile|isPhone|isTablet|mobileLayout|phoneLayout|tabletLayout)\b/g, 'runtime mobile-layout branch'],
    [/safe-area-inset|-webkit-touch-callout|-webkit-overflow-scrolling|\b\d+(?:\.\d+)?dvh\b/g, 'mobile viewport or safe-area compatibility'],
    [/@media\s*\(\s*(?:pointer\s*:\s*coarse|hover\s*:\s*none)\s*\)/g, 'input-device-specific media query'],
    [/\b(?:maxTouchPoints|ontouchstart)\b/g, 'touch detection for renderer layout'],
  ];
  if (connectionSources.test(file)) {
    patterns.push([/\buserAgent\b|\b(?:mobile|phone|tablet):/g, 'client-device layout selection in connection management']);
  }
  for (const [pattern, reason] of patterns) {
    for (const match of source.matchAll(pattern)) failures.push({ offset: match.index, reason, value: match[0] });
  }
  for (const match of terminologySource(file, source).matchAll(clientTerms)) {
    failures.push({ offset: match.index, reason: 'mobile-specific renderer code', value: match[0] });
  }
  for (const match of source.matchAll(/@media\s*\(\s*max-width\s*:\s*(\d+)px\s*\)/g)) {
    if (Number(match[1]) < minimumWidth) failures.push({
      offset: match.index,
      reason: `viewport breakpoint ${match[1]}px is below the supported ${minimumWidth}px minimum`,
      value: match[0],
    });
  }
  return failures.map(({ offset, reason, value }) => ({
    line: source.slice(0, offset).split('\n').length, reason, value,
  }));
}
