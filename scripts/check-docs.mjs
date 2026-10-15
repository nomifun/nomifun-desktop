#!/usr/bin/env node

// Check repository-owned documentation without reading vendor or shipped Skill fixtures.
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const files = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'], {
  cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024,
}).split('\0').filter(path => /\.(?:md|mdc)$/.test(path)
  && !path.startsWith('vendor/') && !path.includes('/vendor/') && !path.includes('/assets/builtin-skills/')
  && !path.includes('/tests/fixtures/') && existsSync(resolve(root, path)));

function prose(path) {
  // Skip fenced examples: their sample URLs and Markdown are not document links.
  let fence;
  return readFileSync(path, 'utf8').split(/\r?\n/).map(line => {
    const marker = /^ {0,3}(`{3,}|~{3,})(.*)$/.exec(line);
    if (!fence && marker) { fence = marker[1]; return ''; }
    if (fence) {
      if (marker && marker[1][0] === fence[0] && marker[1].length >= fence.length && !marker[2].trim()) fence = undefined;
      return '';
    }
    return line;
  }).join('\n');
}

function headingSlug(text) {
  return text.toLowerCase().replace(/<[^>]+>/g, '')
    .replace(/[^\p{L}\p{N}_\s-]/gu, '').replace(/\s/g, '-');
}

const anchorCache = new Map();
function anchors(path) {
  if (anchorCache.has(path)) return anchorCache.get(path);
  const text = prose(path);
  const ids = new Set([...text.matchAll(/\b(?:id|name)=["']([^"']+)["']/g)].map(match => match[1]));
  const used = new Set();
  for (const match of text.matchAll(/^#{1,6}\s+(.+)$/gm)) {
    const base = headingSlug(match[1].replace(/\s+#+\s*$/, '').trim());
    let id = base;
    for (let suffix = 1; used.has(id); suffix++) id = `${base}-${suffix}`;
    used.add(id);
    ids.add(id);
  }
  anchorCache.set(path, ids);
  return ids;
}

const failures = [];
let linkCount = 0;
for (const path of files) {
  const text = prose(resolve(root, path)).replace(/(`+)[\s\S]*?\1/g, '');
  const links = [
    ...[...text.matchAll(/!?\[[^\]\n]*\]\((?:<([^>\n]+)>|([^\s)]+))(?:\s+["'][^\n]*?["'])?\)/g)].map(match => match[1] ?? match[2]),
    ...[...text.matchAll(/\b(?:src|href)=["']([^"']+)["']/g)].map(match => match[1]),
    ...[...text.matchAll(/^\s*\[[^\]]+\]:\s*(?:<([^>\n]+)>|([^\s]+))/gm)].map(match => match[1] ?? match[2]),
  ];
  for (const url of links) {
    if (/^(?:[a-z][\w+.-]*:|\/\/)/i.test(url)) continue;
    linkCount++;
    try {
      const [raw, fragment] = url.split('#');
      const clean = decodeURIComponent(raw.split('?')[0]);
      const target = clean
        ? resolve(clean.startsWith('/') ? root : dirname(resolve(root, path)), clean.replace(/^\//, ''))
        : resolve(root, path);
      if (!existsSync(target)) failures.push(`${path}: missing target ${url}`);
      else if (fragment && /\.md$/i.test(target) && !anchors(target).has(decodeURIComponent(fragment))) {
        failures.push(`${path}: missing anchor ${url} (${relative(root, target)})`);
      }
    } catch (error) {
      failures.push(`${path}: invalid link ${url}: ${error.message}`);
    }
  }
}

if (failures.length) {
  console.error(failures.join('\n'));
  console.error(`Documentation check failed: ${failures.length} issue(s).`);
  process.exitCode = 1;
} else {
  console.log(`Documentation check passed: ${files.length} files, ${linkCount} local links and images.`);
}
