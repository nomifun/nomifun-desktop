/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

// Keep this guard local to the companion's presentation. Canonical messages and
// literal examples in code, maths and URLs remain available unchanged.
const FACE_CORE = /[\^＾][_＿ω▽ᴗ﹏.-][\^＾]|T[_ .-]T|Q[_ .-]Q|QAQ|QwQ|>[_.ω▽ᴗ﹏-]<|[oO][_ .-][oO]|ಠ_ಠ|ಥ_ಥ|[ಥ╥]﹏[ಥ╥]|≧[▽∀ωᴗ]≦|￣[▽ω︶ᴗ]￣|[´`｀][ω▽∀ᴗ][´`｀]|[•・･][̀́]?[ωᴗ▽∀]-|[•・･❛◕°][̀́]?[ _ωᴗ▽∀‿﹏︿ㅂ.-]{1,3}[•・･❛◕°][̀́]?/u;
const FACE_TRIMMINGS = /^[\s*~～.。·｡°ﾟ´`｀'’‘"“”˘¯￣･・•̀́๑٩۶งづっςノﾉつっヾ╰╯]+$/u;
const BARE_FACE = /\^[_ω▽ᴗ﹏.-]\^|T_T|Q_Q|QAQ|QwQ|>_<|ಠ_ಠ|ಥ_ಥ/gu;
const DECORATIVE_IMAGE = /表情|颜文字|^(?:微笑|大笑|捂脸|开心|笑脸|笑)$/u;
const ENGLISH_DECORATIVE_IMAGE = /(?:^|[^a-z])(?:emojis?|emoticons?|emotes?|stickers?|memes?)(?=$|[^a-z])/iu;
const EMOJI = /[#*0-9]\uFE0F?\u20E3|\p{Extended_Pictographic}(?:\uFE0F|\p{Emoji_Modifier})?(?:\u200D\p{Extended_Pictographic}(?:\uFE0F|\p{Emoji_Modifier})?)*(?:[\u{E0020}-\u{E007F}]+)?|\p{Regional_Indicator}|\p{Emoji_Modifier}|[\uFE0F\u20E3\u{E0020}-\u{E007F}]/gu;

// Recognize literal spans before filtering prose. An unfinished code fence is
// already code while it streams; decorative image syntax is the sole mutable
// token here. Ordinary documentation images remain ordinary Markdown.
const LITERAL_OR_IMAGE = /```[\s\S]*?(?:```|$)|~~~[\s\S]*?(?:~~~|$)|`[^`\n]*`|\$\$[\s\S]*?(?:\$\$|$)|\$[^$\n]+\$|!?\[[^\]\n]*\]\((?:\\.|[^()\\\n]|\([^()\n]*\))*\)|!?\[[^\]\n]*\]\[[^\]\n]*\]|<img\b[^>\n]*>|https?:\/\/[^\s<>]+/giu;

function stripProseDecorations(text: string): string {
  return text
    // Explicit sticker annotations are not ordinary parenthetical speech.
    .replace(/\[(?:表情包?|颜文字|贴纸|emoji|emoticon|sticker|meme)\s*[:：][^\]\n]{0,80}\]|【(?:表情包?|颜文字|贴纸|emoji|emoticon|sticker|meme)\s*[:：][^】\n]{0,80}】/giu, '')
    .replace(/\[(?:微笑|笑哭|捂脸|偷笑|大笑|呲牙|调皮|眨眼|比心|爱心|害羞|流泪|大哭|委屈|可怜|doge)\]|【(?:微笑|笑哭|捂脸|偷笑|大笑|呲牙|调皮|眨眼|比心|爱心|害羞|流泪|大哭|委屈|可怜|doge)】/giu, '')
    .replace(/(?<![\w])o\(\*￣▽￣\*\)o(?![\w])/gu, '')
    .replace(/[╰٩ヾ]?[（(]([^()（）\n]{1,48})[)）][╯۶ノﾉづงو✧]*/gu, (whole: string, inner: string) => {
      const face = FACE_CORE.exec(inner);
      if (!face) return whole;
      const rest = inner.slice(0, face.index) + inner.slice(face.index + face[0].length);
      return !rest || FACE_TRIMMINGS.test(rest) ? '' : whole;
    })
    .replace(BARE_FACE, (face: string, at: number, source: string) => {
      // A face embedded in an identifier, filename or path is literal content.
      const before = source[at - 1] ?? '';
      const after = source[at + face.length] ?? '';
      const filenameSuffix = /^\.[A-Za-z0-9]/u.test(source.slice(at + face.length));
      return /[\w./\\]/u.test(before) || /[\w/\\]/u.test(after) || filenameSuffix ? face : '';
    })
    .replace(/(^|\s)(?:[:;=8][-^']?[)(DPp/\\]|<3)(?=$|\s|[，。！？、！?])/gu, '$1')
    .replace(EMOJI, emoji => /^(?:©|®|™)$/u.test(emoji) ? emoji : '');
}

/** Remove nonverbal reply decoration without rewriting the canonical source. */
export function stripCompanionReplyDecorations(text: string): string {
  let out = '';
  let position = 0;
  for (const match of text.matchAll(LITERAL_OR_IMAGE)) {
    out += stripProseDecorations(text.slice(position, match.index));
    const token = match[0];
    const link = /^\[([^\]]*)\]([\s\S]*)$/u.exec(token);
    if (link) {
      const label = stripProseDecorations(link[1]).trim();
      out += label ? `[${label}]${link[2]}` : '';
      position = match.index + token.length;
      continue;
    }
    const image = token.startsWith('![') || /^<img\b/iu.test(token);
    const alt = /^!\[([^\]]*)\]/u.exec(token)?.[1]
      ?? /\balt\s*=\s*["']([^"']*)["']/iu.exec(token)?.[1] ?? '';
    const decorative = image && (DECORATIVE_IMAGE.test(alt)
      || ENGLISH_DECORATIVE_IMAGE.test(token)
      || /\p{Extended_Pictographic}|\p{Regional_Indicator}/u.test(alt));
    out += decorative ? '' : token;
    position = match.index + token.length;
  }
  out += stripProseDecorations(text.slice(position));
  return out.trim();
}
