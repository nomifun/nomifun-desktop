/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { stripCompanionReplyDecorations as strip } from './companionReplyDecorations';

describe('companion reply decoration presentation guard', () => {
  test.each([
    ['你好(^_^)呀', '你好呀'],
    ['好呀(≧▽≦)！', '好呀！'],
    ['别担心(T_T)，我在。', '别担心，我在。'],
    ['好^_^呀 T_T', '好呀'],
    ['谢谢╰(*°▽°*)╯', '谢谢'],
    ['加油(ง •̀_•́)ง', '加油'],
    ['一起٩(๑❛ᴗ❛๑)۶', '一起'],
    ['好呀o(*￣▽￣*)o', '好呀'],
    ['放心QAQ QwQ (ಥ﹏ಥ)(╥﹏╥)', '放心'],
    ['你好（＾▽＾）(*´ω｀*)(｡•̀ᴗ-)✧', '你好'],
    ['收到 :)', '收到'],
    ['收到 :-) :D ;) :P', '收到'],
    ['你好🧑🏽‍💻🇨🇳1️⃣呀', '你好呀'],
    ['👍🏿 ❤️ ☺️ 🌱', ''],
    [`旗帜🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F}。`, '旗帜。'],
    ['好的[表情包：开心]。【表情：微笑】', '好的。'],
    ['[微笑]你好[笑哭]呀【捂脸】', '你好呀'],
    ['好![笑](https://example.test/happy.gif)呀', '好呀'],
    ['好![图片](https://example.test/stickers/hi.webp)呀', '好呀'],
    ['好![😊](https://example.test/happy.png)呀', '好呀'],
    ['好![笑][face]呀', '好呀'],
    ['好![笑](https://example.test/happy(v2).gif)呀', '好呀'],
    ['好<img alt="微笑" src="https://example.test/happy.gif">呀', '好呀'],
    ['[文档😊](https://example.test/T_T)', '[文档](https://example.test/T_T)'],
    ['[😊](https://example.test/happy)', ''],
  ])('removes decorations from %s', (source, expected) => {
    expect(strip(source)).toBe(expected);
  });

  test.each([
    '（附录2）和 (x + y)，编号 [2]。',
    'foo_bar T_T.rs aT_Tb /T_T/ 3.5 25℃ ©2026 ™',
    'https://example.test/T_T?q=^_^&face=:)',
    '![架构图](https://example.test/architecture.png)',
    '![开心果配方](https://example.test/recipe.png)',
    '![Memento 设计模式](https://example.test/memento.png)',
    '![Remote architecture](https://example.test/remote.png)',
    '[微笑](https://example.test/words/smile) 和 [微笑][word]',
    '**重点**\n\n- 第一点\n- 第二点',
    '`const face = "(^_^) 😊";`',
    '```ts\nconst face = "(^_^) 😊";\n```',
    '```ts\nconst face = "(^_^) 😊";',
    '$T_T + x_y$ 和 $$x = (x + y)$$',
    '中文（这里是普通正文）继续。',
  ])('preserves ordinary content and literal examples in %s', source => {
    expect(strip(source)).toBe(source);
  });
});
