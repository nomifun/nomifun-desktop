# WeChat iLink outbound media-upload — discovery findings (Phase C1)

**Date:** 2026-07-09
**Status:** Characterized at the flow level; NOT implemented. Gated on (a) the exact outbound field schema from a reference SDK and (b) a live WeChat bot to verify. Kept as a follow-up; WeChat's `send_media` stays the graceful default no-op until then.

## Why gated (decision gate outcome)

The plan's Phase C1 spike required characterizing the iLink outbound media contract "with confidence." The **flow** is now known from authoritative reverse-engineered community sources, but two things block a responsible implementation right now:
1. The **exact outbound image `item_list` field names** and the **`getuploadurl` response schema** are only fully specified in third-party SDK source (`demo.mjs` etc.), not in text I could extract here (the doc fetch was blocked as a reverse-engineered-protocol topic).
2. WeChat media send **cannot be unit-tested** — it needs a live logged-in bot + WeChat CDN round-trip, which isn't available in this environment.

Implementing it blind would mean guessing field names and shipping untested crypto — against the "no placeholders / don't guess" rule. So it is documented and deferred, not faked.

## The contract (flow, confirmed)

Gateway `https://ilinkai.weixin.qq.com`, CDN `https://novac2c.cdn.weixin.qq.com`. This is Tencent's **official** personal-account Bot API (OpenClaw ClawBot / iLink protocol) — legal, server-side, supports images/voice/files/video natively. Our code already integrates the text path and already **parses inbound** encrypted media (`weixin/types.rs:96-141` `MediaItemData { media: { encrypt_query_param, aes_key }, aeskey, file_name }`) — outbound is symmetric.

Outbound image flow:
1. Generate a random **AES-128** key.
2. **AES-128-ECB** encrypt the image bytes (all iLink CDN media is AES-128-ECB encrypted).
3. `POST /ilink/bot/getuploadurl` with `{ filekey, md5, len }` → a pre-signed CDN upload URL (+ reference params).
4. **PUT** the encrypted bytes to that CDN URL.
5. `POST /ilink/bot/sendmessage` with the standard `msg` wrapper (`message_type: 2`, `message_state: 2`, **`context_token` from the inbound message — required**) and an `item_list` entry of **`type: 2` (ITEM_TYPE_IMAGE)** carrying the base64 `aes_key` + the CDN reference params (mirrors the inbound `MediaItemData` shape).

Headers already implemented for text send apply (`AuthorizationType: ilink_bot_token`, `Authorization: Bearer <bot_token>`, `X-WECHAT-UIN`). `context_token` is already captured per-chat in `weixin/plugin.rs` (`self.context_tokens`).

## What implementation would take (ready-to-do once schema confirmed)

- **New dependency:** ECB mode. `aes` 0.8 is in the tree but the `ecb` crate is not; add `ecb` (or drive `aes` block-by-block with PKCS7) to the `weixin` feature. `base64` + `uuid` are already `weixin` deps.
- **`weixin/types.rs`:** add outbound `SendImageItem` (fields: `aeskey`, CDN reference — copy names from the inbound `MediaItemData`/`MediaEncryptInfo` and a reference SDK) and extend `SendMessageItem` with optional `image_item`/`file_item` (currently text-only, `types.rs:164-170`). `ITEM_TYPE_IMAGE=2`/`FILE=4` constants already exist (`types.rs:8-12`).
- **`weixin/api.rs`:** add `get_upload_url(filekey, md5, len)`, a CDN `PUT`, and `send_image(to_user_id, image_ref, context_token)`; add the AES-128-ECB encrypt helper.
- **`weixin/plugin.rs`:** override `send_media` — encrypt `media.bytes` → getuploadurl → PUT → send_image (reuse `self.context_tokens.get(chat_id)`).
- **Verify:** live WeChat bot round-trip (no automated test possible).

## Reference sources (reverse-engineered community SDKs with exact schemas)

- iLink Bot API protocol — https://www.wechatbot.dev/zh/protocol
- openclaw-weixin `weixin-bot-api.md` (full curl + payloads) — https://github.com/hao-ji-xing/openclaw-weixin/blob/main/weixin-bot-api.md
- XTmai/WeChat-iLinkBot (Python: QR login + send text/file/image) — https://github.com/XTmai/WeChat-iLinkBot
- x1ah/wechat-ilink-demo (Node `demo.mjs`, independent iLink calls) — https://github.com/x1ah/wechat-ilink-demo

The `demo.mjs` / `weixin-bot-api.md` in those repos carry the exact image `item_list` JSON and `getuploadurl` request/response — read one of them to fill `SendImageItem` field names before implementing.
