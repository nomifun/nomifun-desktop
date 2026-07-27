use std::time::Duration;

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};
use base64::Engine;
use md5::{Digest, Md5};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::Client;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::constants::WEIXIN_API_TIMEOUT;
use crate::error::ChannelError;

use super::types::{
    GetUpdatesRequest, GetUpdatesResponse, GetUploadUrlRequest, GetUploadUrlResponse, ILinkResponse, ITEM_TYPE_FILE,
    ITEM_TYPE_IMAGE, ITEM_TYPE_TEXT, QrCodeData, QrCodeStatusData, SendCdnMedia, SendFileItem, SendImageItem,
    SendMessageItem, SendMessageMsg, SendMessageRequest, SendMessageResponse, SendTextItem, UPLOAD_MEDIA_TYPE_FILE,
    UPLOAD_MEDIA_TYPE_IMAGE,
};

const ILINK_APP_ID: &str = "bot";

/// AES-128-ECB ciphertext size for `n` plaintext bytes (PKCS7 always pads, so a
/// full-block plaintext still grows by one block).
fn aes_ecb_padded_size(n: usize) -> usize {
    n + (16 - n % 16)
}

/// Encrypt with AES-128-ECB + PKCS7 padding — the scheme all WeChat CDN media
/// uses. ECB = each 16-byte block encrypted independently (no IV/chaining).
fn aes128_ecb_pkcs7_encrypt(plaintext: &[u8], key: &[u8; 16]) -> Vec<u8> {
    let cipher = Aes128::new(GenericArray::from_slice(key));
    let pad = 16 - (plaintext.len() % 16); // PKCS7: 1..=16 bytes, value == count
    let mut buf = Vec::with_capacity(plaintext.len() + pad);
    buf.extend_from_slice(plaintext);
    buf.extend(std::iter::repeat(pad as u8).take(pad));
    for chunk in buf.chunks_mut(16) {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }
    buf
}

/// Official wire format: random uint32 -> decimal UTF-8 -> base64.
fn encode_wechat_uin(value: u32) -> String {
    base64::engine::general_purpose::STANDARD.encode(value.to_string().as_bytes())
}

fn random_wechat_uin() -> Result<String, ChannelError> {
    let mut bytes = [0u8; 4];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| ChannelError::PlatformApi(format!("failed to generate X-WECHAT-UIN: {error}")))?;
    Ok(encode_wechat_uin(u32::from_be_bytes(bytes)))
}

/// Encode semver as the iLink uint32 client version (0x00MMNNPP).
fn ilink_client_version(version: &str) -> u32 {
    let mut parts = version
        .split_once('-')
        .map_or(version, |(core, _)| core)
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(0) & 0xff);
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major << 16) | (minor << 8) | patch
}

fn base_info() -> serde_json::Value {
    serde_json::json!({
        "channel_version": env!("CARGO_PKG_VERSION"),
        "bot_agent": format!("NomiFun/{}", env!("CARGO_PKG_VERSION")),
    })
}

fn common_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("ilink-app-id"),
        HeaderValue::from_static(ILINK_APP_ID),
    );
    headers.insert(
        HeaderName::from_static("ilink-app-clientversion"),
        HeaderValue::from_str(&ilink_client_version(env!("CARGO_PKG_VERSION")).to_string())
            .expect("packed client version is always a valid header"),
    );
    headers
}

fn authenticated_headers(bot_token: &str) -> Result<HeaderMap, ChannelError> {
    let wechat_uin = random_wechat_uin()?;
    authenticated_headers_with_uin(bot_token, &wechat_uin)
}

fn authenticated_headers_with_uin(bot_token: &str, wechat_uin: &str) -> Result<HeaderMap, ChannelError> {
    let mut headers = common_headers();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(
        HeaderName::from_static("authorizationtype"),
        HeaderValue::from_static("ilink_bot_token"),
    );
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", bot_token.trim()))
            .map_err(|error| ChannelError::InvalidConfig(format!("invalid WeChat bot token header: {error}")))?,
    );
    headers.insert(
        HeaderName::from_static("x-wechat-uin"),
        HeaderValue::from_str(wechat_uin)
            .map_err(|error| ChannelError::InvalidConfig(format!("invalid X-WECHAT-UIN header: {error}")))?,
    );
    Ok(headers)
}

fn ensure_send_message_success(response: &SendMessageResponse) -> Result<(), ChannelError> {
    let ret = response.ret.unwrap_or(0);
    let errcode = response.errcode.unwrap_or(0);
    if ret == 0 && errcode == 0 {
        return Ok(());
    }

    Err(ChannelError::MessageSendFailed(format!(
        "sendmessage API error: ret={ret}, errcode={errcode}, errmsg={}",
        response.errmsg.as_deref().unwrap_or("(none)")
    )))
}

/// HTTP client for the WeChat iLink Bot API.
pub(crate) struct WeixinApi {
    client: Client,
    base_url: String,
    bot_token: String,
}

impl WeixinApi {
    pub fn new(client: Client, base_url: &str, bot_token: &str) -> Self {
        let base = base_url.trim_end_matches('/');

        Self {
            client,
            base_url: base.to_string(),
            bot_token: bot_token.to_string(),
        }
    }

    #[cfg(test)]
    pub fn bot_token(&self) -> &str {
        &self.bot_token
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    async fn authenticated_post<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        body: &impl Serialize,
        timeout: Duration,
    ) -> Result<T, ChannelError> {
        let url = format!("{}/{}", self.base_url, endpoint);
        let headers = authenticated_headers(&self.bot_token)?;

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .timeout(timeout)
            .json(body)
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("{endpoint} request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ChannelError::PlatformApi(format!("{endpoint} HTTP {status}: {text}")));
        }

        resp.json()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("{endpoint} parse failed: {e}")))
    }

    async fn ilink_get<T: DeserializeOwned>(&self, endpoint: &str, query: &[(&str, &str)]) -> Result<T, ChannelError> {
        let url = format!("{}/{}", self.base_url, endpoint);

        let resp = self
            .client
            .get(&url)
            .headers(common_headers())
            .query(query)
            .send()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("{endpoint} request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ChannelError::PlatformApi(format!("{endpoint} HTTP {status}: {text}")));
        }

        resp.json()
            .await
            .map_err(|e| ChannelError::PlatformApi(format!("{endpoint} parse failed: {e}")))
    }

    // -----------------------------------------------------------------------
    // QR code login
    // -----------------------------------------------------------------------

    /// Fetch a QR code for bot login.
    ///
    /// `GET /ilink/bot/get_bot_qrcode?bot_type=3`
    pub async fn get_bot_qrcode(&self) -> Result<QrCodeData, ChannelError> {
        debug!("Fetching WeChat QR code");

        // Try direct response first, then wrapped
        let result: Result<QrCodeData, _> = self.ilink_get("ilink/bot/get_bot_qrcode", &[("bot_type", "3")]).await;

        match result {
            Ok(data) if data.qrcode.is_some() => Ok(data),
            _ => {
                let wrapped: ILinkResponse<QrCodeData> =
                    self.ilink_get("ilink/bot/get_bot_qrcode", &[("bot_type", "3")]).await?;
                wrapped
                    .data
                    .ok_or_else(|| ChannelError::PlatformApi("get_bot_qrcode returned no data".into()))
            }
        }
    }

    /// Check the status of a QR code scan.
    ///
    /// `GET /ilink/bot/get_qrcode_status?qrcode=<ticket>`
    pub async fn get_qrcode_status(&self, qrcode: &str) -> Result<QrCodeStatusData, ChannelError> {
        // Try direct response first, then wrapped
        let result: Result<QrCodeStatusData, _> = self
            .ilink_get("ilink/bot/get_qrcode_status", &[("qrcode", qrcode)])
            .await;

        match result {
            Ok(data) if data.status.is_some() => Ok(data),
            _ => {
                let wrapped: ILinkResponse<QrCodeStatusData> = self
                    .ilink_get("ilink/bot/get_qrcode_status", &[("qrcode", qrcode)])
                    .await?;
                wrapped
                    .data
                    .ok_or_else(|| ChannelError::PlatformApi("get_qrcode_status returned no data".into()))
            }
        }
    }

    // -----------------------------------------------------------------------
    // Long-polling
    // -----------------------------------------------------------------------

    /// Long-poll for new updates using buffer-based protocol.
    ///
    /// `POST /ilink/bot/getupdates`
    pub async fn get_updates(
        &self,
        buf: &str,
        long_poll_timeout: Duration,
    ) -> Result<GetUpdatesResponse, ChannelError> {
        let body = GetUpdatesRequest {
            get_updates_buf: buf.to_string(),
            base_info: base_info(),
        };

        let timeout = long_poll_timeout + Duration::from_secs(10);

        self.authenticated_post("ilink/bot/getupdates", &body, timeout).await
    }

    // -----------------------------------------------------------------------
    // Send message
    // -----------------------------------------------------------------------

    /// Send a text message.
    ///
    /// `POST /ilink/bot/sendmessage`
    pub async fn send_message(
        &self,
        to_user_id: &str,
        text: &str,
        context_token: Option<&str>,
    ) -> Result<(), ChannelError> {
        debug!(to_user_id, "Sending WeChat message");

        let body = SendMessageRequest {
            msg: SendMessageMsg {
                to_user_id: to_user_id.to_string(),
                client_id: Uuid::new_v4().to_string(),
                message_type: 2,
                message_state: 2,
                item_list: vec![SendMessageItem {
                    item_type: ITEM_TYPE_TEXT,
                    text_item: Some(SendTextItem { text: text.to_string() }),
                    image_item: None,
                    file_item: None,
                }],
                context_token: context_token.map(String::from),
            },
            base_info: base_info(),
        };

        let response: SendMessageResponse = self
            .authenticated_post("ilink/bot/sendmessage", &body, WEIXIN_API_TIMEOUT)
            .await
            .map_err(|e| {
                warn!(to_user_id, error = %e, "sendmessage failed");
                ChannelError::MessageSendFailed(format!("sendmessage failed: {e}"))
            })?;

        ensure_send_message_success(&response).map_err(|error| {
            warn!(to_user_id, error = %error, "sendmessage API rejected request");
            error
        })
    }

    // -----------------------------------------------------------------------
    // Send media (image / file) — AES-128-ECB CDN upload + sendmessage
    // -----------------------------------------------------------------------

    /// Upload `bytes` to the WeChat CDN (AES-128-ECB encrypted) and send it as an
    /// image or file message. Mirrors the iLink reference SDK flow exactly:
    /// md5(plaintext) → reserve upload URL → encrypt → PUT to CDN → sendmessage
    /// with a media item referencing the returned encrypted param.
    ///
    /// `context_token` (from the inbound message) is required for the reply to
    /// route to the right conversation — same contract as text sends.
    pub async fn send_media(
        &self,
        to_user_id: &str,
        bytes: Vec<u8>,
        file_name: &str,
        is_image: bool,
        context_token: Option<&str>,
    ) -> Result<(), ChannelError> {
        // 1. Plaintext hash + sizes.
        let rawsize = bytes.len() as u64;
        let rawfilemd5 = {
            let mut hasher = Md5::new();
            hasher.update(&bytes);
            hex::encode(hasher.finalize())
        };
        let filesize = aes_ecb_padded_size(bytes.len()) as u64;

        // 2. Random 16-byte filekey + AES-128 key (hex-encoded on the wire).
        let mut filekey_bytes = [0u8; 16];
        let mut aeskey_bytes = [0u8; 16];
        getrandom::getrandom(&mut filekey_bytes).expect("RNG failure");
        getrandom::getrandom(&mut aeskey_bytes).expect("RNG failure");
        let filekey = hex::encode(filekey_bytes);
        let aeskey_hex = hex::encode(aeskey_bytes);

        let media_type = if is_image {
            UPLOAD_MEDIA_TYPE_IMAGE
        } else {
            UPLOAD_MEDIA_TYPE_FILE
        };

        // 3. Reserve a pre-signed CDN upload URL.
        let upload_req = GetUploadUrlRequest {
            filekey: filekey.clone(),
            media_type,
            to_user_id: to_user_id.to_string(),
            rawsize,
            rawfilemd5,
            filesize,
            no_need_thumb: true,
            aeskey: aeskey_hex.clone(),
            base_info: base_info(),
        };
        let upload_resp: GetUploadUrlResponse = self
            .authenticated_post("ilink/bot/getuploadurl", &upload_req, WEIXIN_API_TIMEOUT)
            .await
            .map_err(|e| ChannelError::MessageSendFailed(format!("getuploadurl failed: {e}")))?;
        // The live iLink API returns a ready-to-use CDN URL (`upload_full_url`,
        // with encrypted_query_param + filekey + taskid embedded), NOT a bare
        // `upload_param` to reconstruct a URL from. Verified against the live
        // gateway. `upload_param` kept only as a legacy fallback.
        let upload_url = upload_resp
            .upload_full_url
            .or(upload_resp.upload_param)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ChannelError::MessageSendFailed("getuploadurl returned no upload_full_url".into()))?;

        // 4. AES-128-ECB encrypt and POST the ciphertext to the returned CDN URL.
        let ciphertext = aes128_ecb_pkcs7_encrypt(&bytes, &aeskey_bytes);
        let download_param = self.upload_to_cdn(&upload_url, ciphertext).await?;

        // 5. Build the media item and send. `media.aes_key` = base64 of the AES
        //    key's HEX STRING bytes (32 ASCII chars), NOT the raw 16 bytes.
        //    LIVE-VERIFIED against the real iLink gateway: this encoding renders a
        //    clean image in WeChat; base64(raw 16 bytes) renders garbled. Keep it.
        let aes_key_field = base64::engine::general_purpose::STANDARD.encode(aeskey_hex.as_bytes());
        let media = SendCdnMedia {
            encrypt_query_param: download_param,
            aes_key: aes_key_field,
            encrypt_type: 1,
        };
        let item = if is_image {
            SendMessageItem {
                item_type: ITEM_TYPE_IMAGE,
                text_item: None,
                image_item: Some(SendImageItem { media, mid_size: filesize }),
                file_item: None,
            }
        } else {
            SendMessageItem {
                item_type: ITEM_TYPE_FILE,
                text_item: None,
                image_item: None,
                file_item: Some(SendFileItem {
                    media,
                    file_name: file_name.to_string(),
                    len: rawsize.to_string(),
                }),
            }
        };

        let body = SendMessageRequest {
            msg: SendMessageMsg {
                to_user_id: to_user_id.to_string(),
                client_id: Uuid::new_v4().to_string(),
                message_type: 2,
                message_state: 2,
                item_list: vec![item],
                context_token: context_token.map(String::from),
            },
            base_info: base_info(),
        };
        let response: SendMessageResponse = self
            .authenticated_post("ilink/bot/sendmessage", &body, WEIXIN_API_TIMEOUT)
            .await
            .map_err(|e| {
                warn!(to_user_id, error = %e, "send media message failed");
                ChannelError::MessageSendFailed(format!("send media message failed: {e}"))
            })?;

        ensure_send_message_success(&response).map_err(|error| {
            warn!(to_user_id, error = %error, "send media message API rejected request");
            error
        })
    }

    /// POST AES-encrypted `ciphertext` to the CDN `upload_url` returned by
    /// `getuploadurl` (it already carries encrypted_query_param + filekey +
    /// taskid) and return the download `x-encrypted-param` used to reference the
    /// file in a message. No gateway auth — the URL is itself the pre-signed
    /// credential. Verified live: POST + octet-stream body → 200 + the header.
    /// Retries transient failures (a live 5xx was observed and cleared on retry).
    async fn upload_to_cdn(&self, upload_url: &str, ciphertext: Vec<u8>) -> Result<String, ChannelError> {
        const MAX_ATTEMPTS: usize = 3;
        let mut last_err = String::new();
        for attempt in 1..=MAX_ATTEMPTS {
            match self
                .client
                .post(upload_url)
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .timeout(WEIXIN_API_TIMEOUT)
                .body(ciphertext.clone())
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    if let Some(param) = resp.headers().get("x-encrypted-param").and_then(|v| v.to_str().ok()) {
                        return Ok(param.to_owned());
                    }
                    last_err = "response missing x-encrypted-param".into();
                }
                Ok(resp) => {
                    let status = resp.status();
                    let text = resp.text().await.unwrap_or_default();
                    last_err = format!("HTTP {status}: {text}");
                }
                Err(e) => last_err = format!("request failed: {e}"),
            }
            if attempt < MAX_ATTEMPTS {
                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            }
        }
        Err(ChannelError::MessageSendFailed(format!(
            "CDN upload failed after {MAX_ATTEMPTS} attempts: {last_err}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aes_ecb_padded_size_matches_reference() {
        // PKCS7 always pads; ceil((n+1)/16)*16.
        assert_eq!(aes_ecb_padded_size(0), 16);
        assert_eq!(aes_ecb_padded_size(15), 16);
        assert_eq!(aes_ecb_padded_size(16), 32);
        assert_eq!(aes_ecb_padded_size(17), 32);
        assert_eq!(aes_ecb_padded_size(31), 32);
        assert_eq!(aes_ecb_padded_size(32), 48);
    }

    #[test]
    fn aes128_ecb_pkcs7_roundtrips() {
        use aes::cipher::BlockDecrypt;
        let key = [7u8; 16];
        let plaintext = b"hello wechat image bytes \x00\x01\x02\xffend".to_vec();
        let ct = aes128_ecb_pkcs7_encrypt(&plaintext, &key);
        assert_eq!(ct.len(), aes_ecb_padded_size(plaintext.len()));
        assert_eq!(ct.len() % 16, 0);

        // Decrypt (ECB block-by-block) and strip PKCS7 to confirm correctness.
        let cipher = Aes128::new(GenericArray::from_slice(&key));
        let mut buf = ct.clone();
        for chunk in buf.chunks_mut(16) {
            cipher.decrypt_block(GenericArray::from_mut_slice(chunk));
        }
        let pad = *buf.last().unwrap() as usize;
        assert!((1..=16).contains(&pad));
        buf.truncate(buf.len() - pad);
        assert_eq!(buf, plaintext);
    }

    #[test]
    fn full_block_plaintext_gets_extra_padding_block() {
        let key = [1u8; 16];
        let plaintext = vec![0xABu8; 16]; // exactly one block
        let ct = aes128_ecb_pkcs7_encrypt(&plaintext, &key);
        assert_eq!(ct.len(), 32, "PKCS7 adds a full padding block on block-aligned input");
    }

    #[test]
    fn api_stores_credentials() {
        let client = Client::new();
        let api = WeixinApi::new(client, "https://ilinkai.weixin.qq.com/", "tok_abc");
        assert_eq!(api.base_url, "https://ilinkai.weixin.qq.com");
        assert_eq!(api.bot_token(), "tok_abc");
    }

    #[test]
    fn api_normalizes_trailing_slash() {
        let client = Client::new();
        let api = WeixinApi::new(client, "https://ilinkai.weixin.qq.com///", "tok");
        assert!(api.base_url.ends_with("com"));
    }

    #[test]
    fn wechat_uin_encodes_decimal_uint32_bytes() {
        let encoded = encode_wechat_uin(305_419_896);
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded).unwrap();
        assert_eq!(String::from_utf8(decoded).unwrap(), "305419896");
    }

    #[test]
    fn random_wechat_uin_decodes_to_uint32_decimal() {
        let encoded = random_wechat_uin().unwrap();
        let decoded = base64::engine::general_purpose::STANDARD.decode(encoded).unwrap();
        let decimal = String::from_utf8(decoded).unwrap();
        assert!(decimal.bytes().all(|byte| byte.is_ascii_digit()));
        decimal.parse::<u32>().unwrap();
    }

    #[test]
    fn client_version_matches_official_packed_semver_format() {
        assert_eq!(ilink_client_version("1.0.11"), 65_547);
        assert_eq!(ilink_client_version("0.3.2-beta.1"), 770);
    }

    #[test]
    fn base_info_identifies_nomifun_version() {
        let info = base_info();
        assert_eq!(info["channel_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(info["bot_agent"], format!("NomiFun/{}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn send_message_business_error_is_not_treated_as_success() {
        let response = SendMessageResponse {
            ret: Some(-1),
            errcode: Some(40003),
            errmsg: Some("invalid context".into()),
        };
        let error = ensure_send_message_success(&response).unwrap_err();
        assert!(error.to_string().contains("ret=-1"));
        assert!(error.to_string().contains("errcode=40003"));
        assert!(ensure_send_message_success(&SendMessageResponse::default()).is_ok());
    }

    #[test]
    fn authenticated_posts_follow_official_headers() {
        let encoded_uin = encode_wechat_uin(123_456_789);
        let headers = authenticated_headers_with_uin("test-token", &encoded_uin).unwrap();
        let expected_version = ilink_client_version(env!("CARGO_PKG_VERSION")).to_string();

        assert_eq!(headers["authorizationtype"], "ilink_bot_token");
        assert_eq!(headers["authorization"], "Bearer test-token");
        assert_eq!(headers["ilink-app-id"], ILINK_APP_ID);
        assert_eq!(
            headers["ilink-app-clientversion"],
            expected_version.as_str()
        );
        assert_eq!(headers["x-wechat-uin"], encoded_uin);
    }
}
