type Segment = { node: Text; start: number; text: string; offsets?: number[] };
type Boundary = { node: Node; position: number; offset: number };
export const safeSubtitleOffset = (text: string, offset: number) => {
  const at = Math.max(0, Math.min(text.length, Math.trunc(offset)));
  return at > 0 && at < text.length && /[\uD800-\uDBFF]/.test(text[at - 1]) && /[\uDC00-\uDFFF]/.test(text[at]) ? at - 1 : at;
};

/** One canonical newline/UTF-16 map for browser Enter markup, saves and cursors. */
export function subtitleTextMap(root: HTMLElement) {
  const segments: Segment[] = [];
  const boundaries: Boundary[] = [];
  const tokens: Array<{ node?: Text; text: string }> = [];
  const chunks: string[] = [];
  let size = 0;
  const append = (text: string, node?: Text) => { chunks.push(text); tokens.push({ node, text }); size += text.length; };
  const block = (node: Node | null) => node instanceof HTMLElement && ['DIV', 'P'].includes(node.tagName);
  const walk = (node: Node) => {
    if (node.nodeType === Node.TEXT_NODE) {
      const original = node.textContent ?? '';
      const text = original.replace(/\r\n?/g, '\n');
      let offsets: number[] | undefined;
      if (original.includes('\r')) {
        offsets = [0]; let length = 0;
        for (let index = 0; index < original.length; index++) {
          if (!(original[index] === '\n' && original[index - 1] === '\r')) length++;
          offsets.push(length);
        }
      }
      segments.push({ node: node as Text, start: size, text, offsets }); append(text, node as Text); return;
    }
    if (node instanceof HTMLElement && node.classList.contains('keyboard-subtitle__partial')) return;
    if (node instanceof HTMLBRElement) {
      const parent = node.parentNode;
      const filler = parent?.childNodes.length === 1 && (parent === root || block(parent))
        || !node.nextSibling && node.previousSibling instanceof HTMLBRElement;
      boundaries.push({ node: parent ?? root, position: Array.from(parent?.childNodes ?? []).indexOf(node), offset: size });
      if (!filler) append('\n');
      return;
    }
    const children = Array.from(node.childNodes);
    children.forEach((child, index) => {
      boundaries.push({ node, position: index, offset: size });
      if (child instanceof HTMLElement && child.classList.contains('keyboard-subtitle__partial')) return;
      if (block(child) && index > 0) append('\n');
      else if (!block(child) && index > 0 && block(children[index - 1])) append('\n');
      walk(child);
    });
    boundaries.push({ node, position: children.length, offset: size });
  };
  walk(root);
  const text = chunks.join('');
  return {
    text, tokens,
    offset(node: Node, position: number) {
      const segment = segments.find(value => value.node === node);
      if (segment) return safeSubtitleOffset(text, segment.start + (segment.offsets?.[position] ?? Math.min(position, segment.text.length)));
      return safeSubtitleOffset(text, boundaries.find(value => value.node === node && value.position === position)?.offset ?? 0);
    },
    locate(offset: number): [Node, number] {
      const at = safeSubtitleOffset(text, offset);
      const segment = segments.find(value => value.text.length > 0 && at >= value.start && at <= value.start + value.text.length);
      if (segment) {
        const local = at - segment.start;
        return [segment.node, segment.offsets ? Math.max(0, segment.offsets.indexOf(local)) : local];
      }
      const boundary = boundaries.find(value => value.offset === at) ?? boundaries.at(-1);
      return boundary ? [boundary.node, boundary.position] : [root, 0];
    },
  };
}

/** Keep existing Text nodes; flatten only browser block/BR wrappers when idle DOM becomes read-only. */
export function patchSubtitleText(root: HTMLElement, desired: string) {
  const before = subtitleTextMap(root);
  if (root.children.length) {
    let index = 0;
    for (const token of before.tokens) {
      const node = token.node ?? document.createTextNode(token.text);
      if (node.data !== token.text) node.data = token.text;
      if (root.childNodes[index] !== node) root.insertBefore(node, root.childNodes[index] ?? null);
      index++;
    }
    while (root.childNodes.length > index) root.lastChild?.remove();
  }
  if (before.text === desired) return;
  const current = subtitleTextMap(root);
  let prefix = 0;
  while (prefix < current.text.length && prefix < desired.length && current.text[prefix] === desired[prefix]) prefix++;
  prefix = safeSubtitleOffset(desired, safeSubtitleOffset(current.text, prefix));
  let suffix = 0;
  while (suffix < current.text.length - prefix && suffix < desired.length - prefix
    && current.text[current.text.length - suffix - 1] === desired[desired.length - suffix - 1]) suffix++;
  while (suffix > 0 && (safeSubtitleOffset(current.text, current.text.length - suffix) !== current.text.length - suffix
    || safeSubtitleOffset(desired, desired.length - suffix) !== desired.length - suffix)) suffix--;
  const oldEnd = current.text.length - suffix;
  const newEnd = desired.length - suffix;
  const range = document.createRange();
  const start = current.locate(prefix); const end = current.locate(oldEnd);
  range.setStart(start[0], start[1]); range.setEnd(end[0], end[1]); range.deleteContents();
  if (newEnd > prefix) range.insertNode(document.createTextNode(desired.slice(prefix, newEnd)));
}
