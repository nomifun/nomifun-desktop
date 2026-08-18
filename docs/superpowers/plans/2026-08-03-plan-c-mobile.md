# nomifun-mobile 实施计划（Plan C）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 uni-app（Vue3+Vite+TS）三端应用 `nomifun-mobile`（Android/iOS/H5），作为 NomiFun 桌面端的极简遥控器：发指令、看状态、收完成反馈、管定时任务。

**Architecture:** UI 层 4 个页面（设备/会话/任务/设置）+ pinia 持久化；核心逻辑全部在 `src/core/`（纯 TS、零 uni 依赖、vitest 可测）：E2E 加密（tweetnacl + @noble/hashes）、RPC、双传输（LAN WS 直连 / 中继 WS），LAN 优先自动回退中继。

**Tech Stack:** uni-app (Vue3 + Vite + TypeScript)、pinia、tweetnacl、@noble/hashes、vitest。H5 为主调试目标（`npm run dev:h5`）；App 端由 HBuilderX 云打包（文档说明，不在任务内）。

## Global Constraints

- 规范：`~/src/nomifun-tauri/docs/superpowers/specs/2026-08-03-bridge-protocol-v1.md`（BINDING）
- 手机端不存过程数据：事件环形缓冲 ≤50 条、每条摘要 ≤8KB；RPC 超时 15s；列表 limit ≤50
- Base64 标准字母表带 padding（QR/桥接串用 base64url no-pad）；device_id = SHA-512(pk) 前 8 字节 hex
- 仓库 `/home/developer/src/nomifun-mobile`；git author `NomiFun Contributor <nomifun@users.noreply.github.com>`；无 AI 署名；禁止 `.github/workflows/`
- `src/core/**` 禁止 import 任何 `uni.*` / Vue —— 平台 API 一律通过注入的接口（`WebSocketLike`、`HttpProbe`、`StorageLike`）

---

### Task C1: 项目脚手架

**Files:**
- Create: 整个项目骨架 + `vitest.config.ts` + `.gitignore` + `README.md`

- [ ] **Step 1: 创建项目**

```bash
cd /home/developer/src
npx degit dcloudio/uni-preset-vue#vite-ts nomifun-mobile
cd nomifun-mobile
git init -b main && git config user.name "NomiFun Contributor" && git config user.email "nomifun@users.noreply.github.com"
```

若 degit 失败（网络/模板变动），手工脚手架：`package.json` 参照 uni-app 官方 vite-ts 模板（deps：`@dcloudio/uni-app`、`@dcloudio/uni-components`、`@dcloudio/uni-h5`、`vue`、`pinia`；devDeps：`@dcloudio/vite-plugin-uni`、`@dcloudio/types`、`vite`、`typescript`、`vue-tsc`；scripts：`"dev:h5": "uni"`、`"build:h5": "uni build"`），`vite.config.ts` 用 `uni()` 插件，`src/main.ts`/`src/App.vue`/`src/pages.json`/`src/manifest.json` 按 uni-app 最小骨架。

- [ ] **Step 2: 加依赖与 vitest**

```bash
npm i tweetnacl @noble/hashes@^1.8 pinia
npm i -D vitest
```

（`@noble/hashes` 必须锁 1.x：v2 移除了 `sha256` 子路径导出且所有子路径 import 需 `.js` 扩展名，会破坏 C3 的导入写法。）

`vitest.config.ts`:

```ts
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: { include: ['tests/**/*.test.ts'], environment: 'node' },
});
```

`package.json` scripts 加 `"test": "vitest run"`。`.gitignore`：`node_modules`、`dist`、`unpackage`。

- [ ] **Step 3: 验证** — Run: `npm run dev:h5 -- --port 5175 &` 稍候 `curl -sf http://localhost:5175 >/dev/null && echo OK; kill %1`；Expected: OK
- [ ] **Step 4: Commit** — `git add -A && git commit -m "chore: scaffold uni-app project with vitest"`

### Task C2: base64 与字节工具（src/core/bytes.ts）

**Interfaces:**
- Produces: `b64encode(b: Uint8Array): string`（标准带 padding）、`b64decode(s: string): Uint8Array`、`b64urlEncode(b: Uint8Array): string`（no-pad）、`b64urlDecode(s: string): Uint8Array`、`utf8encode(s: string): Uint8Array`、`utf8decode(b: Uint8Array): string`、`bytesToHex(b: Uint8Array): string`、`concatBytes(...arrs): Uint8Array`

- [ ] **Step 1: 失败测试**（`tests/core/bytes.test.ts`）

```ts
import { describe, expect, it } from 'vitest';
import { b64decode, b64encode, b64urlDecode, b64urlEncode, bytesToHex, utf8decode, utf8encode } from '../../src/core/bytes';

describe('base64', () => {
  it('roundtrips arbitrary bytes with std alphabet and padding', () => {
    const b = new Uint8Array([0, 1, 2, 250, 251, 252, 253, 254]); // 8 字节：8 % 3 == 2 → 必有 '=' padding
    const s = b64encode(b);
    expect(s.endsWith('=')).toBe(true);
    expect(b64decode(s)).toEqual(b);
  });
  it('matches known vector', () => {
    expect(b64encode(utf8encode('hello'))).toBe('aGVsbG8=');
  });
  it('b64url has no padding and uses -_', () => {
    const b = new Uint8Array([251, 255, 191]);
    const s = b64urlEncode(b);
    expect(s.includes('=')).toBe(false);
    expect(/^[A-Za-z0-9\-_]+$/.test(s)).toBe(true);
    expect(b64urlDecode(s)).toEqual(b);
  });
  it('utf8 roundtrip with cjk', () => {
    expect(utf8decode(utf8encode('任务完成✓'))).toBe('任务完成✓');
  });
  it('hex', () => {
    expect(bytesToHex(new Uint8Array([0xde, 0xad]))).toBe('dead');
  });
});
```

- [ ] **Step 2: 验证失败** — Run: `npm test`；Expected: FAIL
- [ ] **Step 3: 实现**（纯 TS 查表实现 base64 编解码，不用 btoa/atob/Buffer；b64url 由标准结果替换 `+/`→`-_` 去 `=`，解码反向补 padding；utf8 用 `TextEncoder/TextDecoder`，若目标 App webview 无 TextEncoder 则手写 UTF-8 编解码——先用 TextEncoder，加运行时 fallback）
- [ ] **Step 4: 验证通过** — `npm test`
- [ ] **Step 5: Commit** — `git commit -am "feat(core): byte utils with std and url-safe base64"`

### Task C3: E2E 加密（src/core/crypto.ts）+ 互操作向量

**Interfaces:**
- Produces:

```ts
export interface E2EFrame { v: 1; from: string; pk?: string; n: string; c: string }
export interface KeyPair { pk: Uint8Array; sk: Uint8Array }
export function generateKeyPair(): KeyPair;               // nacl.box.keyPair()
export function keyPairFromSecret(sk: Uint8Array): KeyPair;
export function deviceIdFromPk(pk: Uint8Array): string;    // hex(sha512(pk)[0..8])
export function seal(inner: object, from: string, peerPk: Uint8Array, selfSk: Uint8Array, includePk?: Uint8Array, nonce?: Uint8Array): E2EFrame;
export function open(frame: E2EFrame, peerPk: Uint8Array, selfSk: Uint8Array): object | null; // 失败返回 null
export function pairMac(code: string, desktopPk: Uint8Array, mobilePk: Uint8Array): string;   // hex
```

- [ ] **Step 1: 复制互操作向量**

```bash
mkdir -p tests/vectors
cp ~/src/nomifun-tauri/crates/backend/nomifun-bridge/tests/vectors/bridge_e2e_vectors.json tests/vectors/
```

（若源文件尚不存在——Plan A Task A4 未执行——本任务测试中标注 `it.skipIf(!vectorsExist)` 并输出提示，Plan A 完成后回来重跑。）

- [ ] **Step 2: 失败测试**（`tests/core/crypto.test.ts`）

```ts
import { readFileSync, existsSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { b64decode, utf8encode } from '../../src/core/bytes';
import { deviceIdFromPk, keyPairFromSecret, open, pairMac, seal } from '../../src/core/crypto';

const VEC_PATH = 'tests/vectors/bridge_e2e_vectors.json';
const vectors = existsSync(VEC_PATH) ? JSON.parse(readFileSync(VEC_PATH, 'utf8')) : null;

describe('crypto', () => {
  it('seal/open roundtrip and tamper rejection', () => {
    const d = keyPairFromSecret(new Uint8Array(32).fill(1));
    const m = keyPairFromSecret(new Uint8Array(32).fill(2));
    const inner = { kind: 'rpc', ctr: 2, id: 'x', method: 'device.info', params: {} };
    const f = seal(inner, deviceIdFromPk(m.pk), d.pk, m.sk, m.pk);
    expect(f.pk).toBeDefined();
    expect(open(f, m.pk, d.sk)).toEqual(inner);
    const bad = { ...f, c: f.c.slice(0, -4) + 'AAAA' };
    expect(open(bad, m.pk, d.sk)).toBeNull();
  });

  it.skipIf(!vectors)('decrypts rust-generated cipher (interop)', () => {
    const dsk = b64decode(vectors.desktop_sk);
    const mpk = b64decode(vectors.mobile_pk);
    const frame = { v: 1 as const, from: 'x', n: vectors.nonce, c: vectors.cipher_b64 };
    const opened = open(frame, mpk, keyPairFromSecret(dsk).sk);
    expect(opened).toEqual(JSON.parse(vectors.plaintext));
  });

  it.skipIf(!vectors)('pair mac and device ids match rust (interop)', () => {
    const dpk = b64decode(vectors.desktop_pk);
    const mpk = b64decode(vectors.mobile_pk);
    expect(pairMac(vectors.pair_code, dpk, mpk)).toBe(vectors.pair_mac_hex);
    expect(deviceIdFromPk(dpk)).toBe(vectors.device_id_of_desktop_pk);
    expect(deviceIdFromPk(mpk)).toBe(vectors.device_id_of_mobile_pk);
  });
});
```

- [ ] **Step 3: 验证失败** — `npm test`
- [ ] **Step 4: 实现**

```ts
import nacl from 'tweetnacl';
import { hkdf } from '@noble/hashes/hkdf';
import { hmac } from '@noble/hashes/hmac';
import { sha256 } from '@noble/hashes/sha256';
import { b64decode, b64encode, bytesToHex, concatBytes, utf8decode, utf8encode } from './bytes';

const PAIR_SALT = utf8encode('nomifun-bridge-pair-v1');

export function keyPairFromSecret(sk: Uint8Array): KeyPair {
  const kp = nacl.box.keyPair.fromSecretKey(sk);
  return { pk: kp.publicKey, sk: kp.secretKey };
}

export function generateKeyPair(): KeyPair {
  const kp = nacl.box.keyPair();
  return { pk: kp.publicKey, sk: kp.secretKey };
}

export function deviceIdFromPk(pk: Uint8Array): string {
  return bytesToHex(nacl.hash(pk).slice(0, 8));
}

export function seal(inner: object, from: string, peerPk: Uint8Array, selfSk: Uint8Array, includePk?: Uint8Array, nonce?: Uint8Array): E2EFrame {
  const n = nonce ?? nacl.randomBytes(24);
  const c = nacl.box(utf8encode(JSON.stringify(inner)), n, peerPk, selfSk);
  return { v: 1, from, ...(includePk ? { pk: b64encode(includePk) } : {}), n: b64encode(n), c: b64encode(c) };
}

export function open(frame: E2EFrame, peerPk: Uint8Array, selfSk: Uint8Array): object | null {
  try {
    const plain = nacl.box.open(b64decode(frame.c), b64decode(frame.n), peerPk, selfSk);
    if (!plain) return null;
    return JSON.parse(utf8decode(plain));
  } catch {
    return null;
  }
}

export function pairMac(code: string, desktopPk: Uint8Array, mobilePk: Uint8Array): string {
  const prk = hkdf(sha256, utf8encode(code), PAIR_SALT, new Uint8Array(0), 32);
  return bytesToHex(hmac(sha256, prk, concatBytes(desktopPk, mobilePk)));
}
```

- [ ] **Step 5: 验证通过** — `npm test`（互操作用例若向量缺失则 skip，有则必须过）
- [ ] **Step 6: Commit** — `git add -A && git commit -m "feat(core): nacl box e2e crypto with rust interop vectors"`

### Task C4: 协议类型与计数器（src/core/protocol.ts, src/core/counter.ts）

**Interfaces:**
- Produces: `protocol.ts` — TS 类型：`Inner`（联合：`pair_request/pair_ok/pair_err/rpc/rpc_result/event`，字段同协议 §2.2）、`RelayFrame`（联合 register/registered/forward/deliver/error/ping/pong）、RPC method 名与参数/结果 interface（§6）、事件名（§7）、`relayAuth(key: string, id: string, ts: number): string`；`counter.ts` — `class CtrGuard { constructor(last: number); accept(ctr: number): boolean; get last(): number }` 与 `class CtrSource { constructor(last: number); next(): number }`

- [ ] **Step 1: 失败测试**（`tests/core/protocol.test.ts`：`relayAuth('key1','dev1',1700000000000)` 为 64 hex 且确定性；`CtrGuard` 拒绝重放；`CtrSource.next()` 严格递增）
- [ ] **Step 2: 验证失败** — `npm test`
- [ ] **Step 3: 实现**（`relayAuth` = `bytesToHex(hmac(sha256, utf8encode(key), utf8encode(id + ':' + ts)))`；类型定义照协议逐字）
- [ ] **Step 4: 验证通过** — `npm test`
- [ ] **Step 5: Commit** — `git commit -am "feat(core): protocol types, relay auth, counters"`

### Task C5: RPC 待答表（src/core/rpc.ts）

**Interfaces:**
- Produces:

```ts
export class RpcTable {
  constructor(private timeoutMs = 15_000) {}
  create(method: string, params: object): { id: string; promise: Promise<unknown> }; // id = uuid v4（getRandomValues/nacl.randomBytes 实现）
  resolve(id: string, ok: boolean, result?: unknown, error?: { code: string; message: string }): void;
  rejectAll(reason: string): void; // 连接断开时调用
}
```

- [ ] **Step 1: 失败测试**（`tests/core/rpc.test.ts`：resolve ok → promise 兑现；resolve !ok → reject 且 error.code 保留；超时（用 `vi.useFakeTimers()` 快进 15s）→ reject `code:"timeout"`；`rejectAll` → 全部 reject；id 形如 uuid v4 正则）
- [ ] **Step 2: 验证失败** — `npm test`
- [ ] **Step 3: 实现**（Map<id,{resolve,reject,timer}>；uuid v4：16 随机字节改版本位/变体位后格式化）
- [ ] **Step 4: 验证通过** — `npm test`
- [ ] **Step 5: Commit** — `git commit -am "feat(core): rpc pending table with timeout"`

### Task C6: 传输层（src/core/transport.ts）

**Interfaces:**
- Produces:

```ts
export interface WebSocketLike { send(data: string): void; close(): void; onopen/onmessage/onclose/onerror: ... } // 与浏览器 WebSocket 形状兼容
export type WsFactory = (url: string) => WebSocketLike;
export type TransportState = 'connecting' | 'ready' | 'closed';
export interface Transport {
  send(frameJson: string): void;                 // 发送一个 E2EFrame JSON 文本
  onFrame(cb: (frameJson: string) => void): void; // 收到对端 E2EFrame JSON 文本
  onState(cb: (s: TransportState, err?: string) => void): void;
  close(): void;
}
export class LanTransport implements Transport { constructor(wsUrl: string, wsFactory: WsFactory) }
// 直连：send 原样发；onmessage 里 {"type":"ping"} 回 pong、{"type":"pong"} 忽略，其余按 E2EFrame 上抛
export class RelayTransport implements Transport {
  constructor(relayUrl: string, serverKey: string, selfId: string, peerId: string, wsFactory: WsFactory, now: () => number = Date.now)
}
// 中继：open 后发 register(role:"mobile")；收 registered → ready；send 包成 forward{to:peerId}；deliver 解出 frame 上抛；
// error{code:"peer_offline"} → onState('ready', 'peer_offline')（连接保持）；auth_failed → close；30s 定时 ping
```

- [ ] **Step 1: 失败测试**（`tests/core/transport.test.ts`：写一个 `FakeWs` 类记录 sent[]、可手动触发 onopen/onmessage；断言：RelayTransport open 后第一帧是合法 register（校验 auth=relayAuth(...)）；registered 后 state=ready；send('X') 产生 `{"type":"forward","to":"<peer>","frame":X}`；deliver 上抛 frame JSON；peer_offline 触发 onState 回调带 err；LanTransport ping→pong 自动应答且不上抛）
- [ ] **Step 2: 验证失败** — `npm test`
- [ ] **Step 3: 实现**（如接口注释；RelayTransport 的 ping 定时器在 close 时清除）
- [ ] **Step 4: 验证通过** — `npm test`
- [ ] **Step 5: Commit** — `git commit -am "feat(core): lan and relay transports"`

### Task C7: BridgeClient（src/core/client.ts）

**Interfaces:**
- Consumes: crypto/protocol/counter/rpc/transport
- Produces:

```ts
export interface PairedDesktop { id: string; pk: string /*b64*/; name: string; lan?: { host: string; port: number }; relay?: { url: string; key: string }; lastCtrIn: number; lastCtrOut: number }
export interface BridgePayload { v: 1; id: string; pk: string; name: string; lan?: {...}; relay?: {...}; code?: string } // QR/桥接串解析结果
export function parseBridgeString(s: string): BridgePayload | null; // "nomifun-bridge:"+b64url(json)
export interface ClientDeps { wsFactory: WsFactory; identity: KeyPair; selfName: string; platform: string; now?: () => number }
export class BridgeClient {
  constructor(desktop: PairedDesktop, deps: ClientDeps);
  connect(prefer?: 'lan' | 'relay'): void;   // LAN 可用先 LAN，onState closed 且未 ready 过 → 自动回退 relay
  onState(cb: (s: 'connecting'|'ready'|'offline', detail?: string) => void): void;
  onEvent(cb: (name: string, data: unknown) => void): void;
  onCtrChange(cb: (lastCtrIn: number, lastCtrOut: number) => void): void; // 供存储层持久化
  call<T = unknown>(method: string, params?: object): Promise<T>;
  close(): void;
  static async pair(payload: BridgePayload, code: string, deps: ClientDeps, viaLan: boolean): Promise<PairedDesktop>;
  // 完整配对流程，mac 校验失败抛错。返回值必须为 lastCtrOut=1（pair_request 已用 ctr=1）、
  // lastCtrIn=1（pair_ok 已用 ctr=1）——配对后第一条 RPC 必须用 ctr=2，否则桌面按重放丢弃。
}
```

- [ ] **Step 1: 失败测试**（`tests/core/client.test.ts`，FakeWs 直接扮演桌面端：用 vectors/固定密钥在测试内实现"桌面侧" seal/open：
  1. `pair()`：断言发出的帧含 pk 且 inner 为 pair_request(code, ctr=1)；桌面回 pair_ok(mac 正确) → resolve PairedDesktop（pk/name 正确，`lastCtrOut === 1 && lastCtrIn === 1`）；mac 错误 → reject
  2. `call('device.info')`：inner.kind=rpc、**配对后首个 RPC 的 ctr === 2** 且后续严格递增、id 为 uuid；桌面回 rpc_result → promise 兑现
  3. 桌面推 event → onEvent 回调
  4. 重放 event（同 ctr）→ 不触发第二次回调
  5. LAN 连接失败（FakeWs 立即 onclose）→ 自动用 relay 工厂重连（断言第二次 wsFactory 调用的 url 是 relay url）
  6. 连续 10 个无法解密的帧（随机 c）→ client 主动 close transport 且 onState 收到 `('offline','decrypt_failures')`；第 9 个后收到一帧合法帧则计数清零）
- [ ] **Step 2: 验证失败** — `npm test`
- [ ] **Step 3: 实现**（组合各模块；收帧流程：JSON.parse → open(帧, desktopPk, selfSk)（null → 失败计数 +1，连续 10 次 → close + onState('offline','decrypt_failures')；成功则清零）→ CtrGuard.accept(inner.ctr)（false → 同样计数丢弃）→ 按 kind 分发 rpc_result/event；发送：ctrOut.next() 填 ctr → seal → transport.send；每次 ctr 变化触发 onCtrChange；`pair()` 成功时以 lastCtrOut=1/lastCtrIn=1 初始化返回值）
- [ ] **Step 4: 验证通过** — `npm test`
- [ ] **Step 5: Commit** — `git commit -am "feat(core): bridge client with pairing, fallback and replay guard"`

### Task C8: LAN 发现（src/core/discovery.ts）

**Interfaces:**
- Produces:

```ts
export interface HttpProbe { (url: string, timeoutMs: number): Promise<string | null> } // 返回 body 或 null
export interface DiscoveredDesktop { host: string; port: number; id: string; name: string; pk: string; version: string }
export async function probeLan(selfIpHint: string | null, probe: HttpProbe, port = 25810, concurrency = 20, timeoutMs = 300): Promise<DiscoveredDesktop[]>;
// selfIpHint 形如 "192.168.1.23" → 扫 192.168.1.1-254（跳过自身）；null → 返回 []
```

- [ ] **Step 1: 失败测试**（fake probe：仅 `192.168.1.10` 返回 `{"app":"nomifun","v":1,"id":"aa","name":"Desk","pk":"cGs=","version":"0.3.7"}`；断言结果 1 条且字段正确；非 nomifun JSON / 超时(null) 被忽略；并发不超过 20——probe 里计数在飞数断言）
- [ ] **Step 2: 验证失败** — `npm test`
- [ ] **Step 3: 实现**（生成 254 个候选 → 简单 worker-pool 并发扫描 `http://{ip}:{port}/bridge/info`；解析校验 `app === 'nomifun'`）
- [ ] **Step 4: 验证通过** — `npm test`
- [ ] **Step 5: Commit** — `git commit -am "feat(core): lan subnet discovery"`

### Task C9: 平台适配与存储（src/platform/, src/stores/）

**Files:**
- Create: `src/platform/ws.ts`（`wsFactory`：H5 用原生 `WebSocket`；App 端用 `uni.connectSocket` 包装成 `WebSocketLike`——`// #ifdef H5 / APP-PLUS` 条件编译）
- Create: `src/platform/http.ts`（`httpProbe`：`uni.request` 包装，timeout 支持）
- Create: `src/platform/storage.ts`（`StorageLike { get(k): string|null; set(k,v): void; remove(k): void }`，uni.getStorageSync 包装；vitest 下用内存 Map 实现 `MemoryStorage` 导出供测试）
- Create: `src/stores/devices.ts`、`src/stores/settings.ts`、`src/stores/events.ts`（pinia）

**Interfaces:**
- Produces: `useDevicesStore`（`list: PairedDesktop[]`、`add/remove/updateCtrs(id, ctrIn, ctrOut)`，持久化 key `nf.devices`）；`useSettingsStore`（`relayUrl/relayKey`，key `nf.settings`）；`useEventsStore`（`push(evt: {ts, deviceId, name, summary})` 环形 ≤50 条、summary 截断 8KB，key `nf.events`；`clear()`）

- [ ] **Step 1: 失败测试**（`tests/stores/events.test.ts` 用 `createPinia()` + `MemoryStorage`：push 60 条 → 只留最近 50；超长 summary 按 UTF-8 编码字节数截断至 ≤8192 字节（用 `utf8encode(s).length` 校验，CJK 每字 3 字节）；`tests/stores/devices.test.ts`：add/updateCtrs/remove 后重建 store 从 storage 恢复）
- [ ] **Step 2: 验证失败** — `npm test`
- [ ] **Step 3: 实现**（stores 通过参数/依赖注入接收 StorageLike，默认取 uni 包装；测试注入 MemoryStorage）
- [ ] **Step 4: 验证通过** — `npm test`
- [ ] **Step 5: Commit** — `git commit -am "feat: platform adapters and persisted pinia stores"`

### Task C10: 页面——设备 + 配对（pages/devices）

**Files:**
- Create: `src/pages/devices/index.vue`, `src/pages/devices/pair.vue`
- Modify: `src/pages.json`（注册页面 + tabBar 4 项：设备/会话/任务/设置）

**Interfaces:**
- Consumes: `BridgeClient.pair`、`parseBridgeString`、`probeLan`、stores
- Produces: 设备列表（名称、在线徽标——来自全局连接管理器 `src/composables/useConnections.ts`：为每个已配对设备维护一个 BridgeClient 单例，暴露 `stateOf(id)`、`clientOf(id)`）；添加设备页：三种入口（① App 扫码 `uni.scanCode`（`// #ifdef APP-PLUS`）② 粘贴桥接串 + 输入配对码 ③ LAN 扫描列表点选 + 输入配对码）→ `BridgeClient.pair` → 存入 devices store → 返回列表

- [ ] **Step 1: 实现 useConnections + 两个页面**（Vue SFC，uni 内置组件 view/text/input/button/switch；样式极简 scoped css）
- [ ] **Step 2: 手动验证** — Run: `npm run dev:h5`，浏览器打开：设备页可见"添加设备"，粘贴非法桥接串报错提示（`uni.showToast`）
- [ ] **Step 3: Commit** — `git commit -am "feat: devices page with scan/paste/lan pairing"`

### Task C11: 页面——会话（pages/sessions）

**Files:**
- Create: `src/pages/sessions/index.vue`, `src/pages/sessions/detail.vue`
- Modify: `src/pages.json`

**Interfaces:**
- Consumes: `client.call('conversations.list'/'send'/'status'/'result'/'cancel'/'confirmations.list'/'confirmations.confirm')`、`client.onEvent`
- Produces: 列表页（设备选择器 picker + 会话列表：name/status 徽标/is_processing 转圈；下拉刷新 = 重新 list；"新建会话"按钮 → `conversations.create`）；详情页（顶部状态卡片（status/runtime.state，"刷新"调 status）、最近结果卡片（进入时调 `conversations.result`，收到 `task.completed` 事件自动更新并写入 events store）、待确认列表（`conversations.attention` 事件触发拉取 `confirmations.list`，每项渲染 options 按钮 → `confirmations.confirm`）、底部输入框 + 发送（`conversations.send`，发送后显示 msg_id 已接受/完成态）、取消按钮（运行中时可见））

- [ ] **Step 1: 实现两个页面**
- [ ] **Step 2: 手动验证** — `npm run dev:h5`：无设备时列表页显示空态引导文案
- [ ] **Step 3: Commit** — `git commit -am "feat: sessions list and detail with send/result/confirmations"`

### Task C12: 页面——定时任务 + 设置（pages/tasks, pages/settings）

**Files:**
- Create: `src/pages/tasks/index.vue`, `src/pages/tasks/edit.vue`, `src/pages/settings/index.vue`
- Modify: `src/pages.json`

**Interfaces:**
- Consumes: `client.call('cron.list'/'create'/'update'/'delete'/'runNow')`、`cron.executed` 事件
- Produces: 任务列表（name/enabled switch（→update patch {enabled}）/next_run_at 时间/last_status；操作：立即执行、编辑、删除（confirm 弹窗）；收到 `cron.executed` 刷新并 toast）；编辑页（name、message、schedule 编辑器：picker 选 kind（at/every/cron）→ at: datetime picker → at_ms；every: 数字分钟 → every_ms；cron: 文本输入 expr + tz 默认留空；保存 → create/update）；设置页（中继 url/key 表单存 settings store、本机身份指纹（deviceIdFromPk 展示）、"清空本地缓存"按钮（events.clear + toast）、"重置身份"危险按钮（confirm 后删除本机密钥——需重新配对））

- [ ] **Step 1: 实现三个页面**
- [ ] **Step 2: 手动验证** — `npm run dev:h5` 四个 tab 均可导航，无控制台错误
- [ ] **Step 3: Commit** — `git commit -am "feat: cron tasks management and settings pages"`

### Task C13: 收尾——README 与 H5 验收清单

**Files:**
- Create: `README.md`, `docs/h5-acceptance.md`

- [ ] **Step 1: README**（中文：项目定位、架构图（core/平台适配/页面三层）、开发 `npm run dev:h5`、构建 `npm run build:h5`、App 端 HBuilderX 云打包指引（导入项目 → manifest 配置 appid → 云打包；`uni.scanCode`/`connectSocket` 权限说明）、协议文档指针、H5 限制说明（https 页面无法混合内容访问 http LAN 探测——建议 H5 走中继或以 http 服务页面））
- [ ] **Step 2: 验收清单**（`docs/h5-acceptance.md`：配对（扫码/粘贴/LAN 扫描×3）、发指令收完成反馈、确认项批准、cron 全 CRUD+runNow、断线回退中继、重放防护（重复事件不重复入库）、存储上限（events≤50））
- [ ] **Step 3: 全量测试** — Run: `npm test`；Expected: 全 PASS（互操作向量存在时不得 skip）
- [ ] **Step 4: 作者合规** — `git log --format='%an <%ae>' | sort -u` 仅 `NomiFun Contributor <nomifun@users.noreply.github.com>`；`ls .github/workflows 2>/dev/null` 为空
- [ ] **Step 5: Commit** — `git add -A && git commit -m "docs: readme and h5 acceptance checklist"`
