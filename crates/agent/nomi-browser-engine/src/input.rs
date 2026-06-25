//! input —— 输入路径：**纯几何**（点击点计算）+ **CDP 输入合成**，全程 **禁 DPR**。
//!
//! 背景（DESIGN §10，设计裁决④）：CDP `Input.dispatchMouseEvent` 吃主框架 viewport 的
//! **CSS 像素**，故输入路径全程不做 deviceScaleFactor 换算（DPR 仅属截图，P3+）。点击点来自
//! `DOM.getContentQuads`（已 CSS 像素，扁平 8 数 = 4 角点）→ 钳位 (0,0)-(innerW,innerH)
//! + 鞋带公式面积 >0.99 过滤退化 quad + 取首个有效 quad 的中点；区分 NotVisible /
//!   NotInViewport。
//!
//! 本模块分两层：
//! - **纯几何函数**（[`quad_to_points`]/[`shoelace_area`]/[`pick_click_point`]/[`clamp_point`]/
//!   [`frame_offset`]，B1 已交付）—— 不发 CDP，不接 chrome，纯算术。
//! - **CDP 输入合成**（B5，本文件后段）—— 收 `&Connection` + session 的自由 async 函数，发裸
//!   CDP `DOM.getContentQuads` / `Runtime.evaluate` / `Input.dispatchMouseEvent` /
//!   `Input.dispatchKeyEvent` / `Input.insertText`。**任何路径都不收 / 不乘 deviceScaleFactor**：
//!   坐标原样从 getContentQuads（CSS 像素）流到 dispatchMouseEvent（CSS 像素）。组合键
//!   （`Ctrl+A` / `Enter` / `Shift+Tab`…）由纯函数 [`parse_key_combo`] 按 **US 布局**合成
//!   key/code/windowsVirtualKeyCode + modifiers 位掩码（CDP 约定：Alt=1, Ctrl=2, Meta=4,
//!   Shift=8）；文本 / IME / secret 走 [`insert_text`]（`Input.insertText`）。所有 send 经
//!   `map_transport_err`，**绝不 panic**。
//!
//! B5 不做 hit-target 串联（B4 已有原语，C1 才串）/ 重试（B6）/ act 动作（C1）/ facade。

use chromiumoxide::cdp::browser_protocol::dom::GetContentQuadsParams;
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchKeyEventParams, DispatchKeyEventType, DispatchMouseEventParams, DispatchMouseEventType,
    InsertTextParams, MouseButton,
};
use chromiumoxide::cdp::js_protocol::runtime::{EvaluateParams, RemoteObjectId};

use crate::backend::cdp::map_transport_err;
use crate::engine::BrowserError;
use crate::transport::Connection;

/// CDP getContentQuads 的一个 quad：扁平 8 数 = 4 角点 (x1,y1,...,x4,y4)，CSS 像素。
pub type Quad = Vec<f64>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// iframe 链上某层的偏移（父帧坐标系下子帧左上角），CSS 像素。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameRect {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeomError {
    NotVisible,
    NotInViewport,
}

/// 扁平 8 数 → 4 角点。长度非 8 返 None。
pub fn quad_to_points(q: &Quad) -> Option<[Point; 4]> {
    if q.len() != 8 {
        return None;
    }
    Some([
        Point { x: q[0], y: q[1] },
        Point { x: q[2], y: q[3] },
        Point { x: q[4], y: q[5] },
        Point { x: q[6], y: q[7] },
    ])
}

/// 鞋带公式面积（绝对值）。
pub fn shoelace_area(p: &[Point; 4]) -> f64 {
    let mut s = 0.0;
    for i in 0..4 {
        let a = p[i];
        let b = p[(i + 1) % 4];
        s += a.x * b.y - b.x * a.y;
    }
    (s / 2.0).abs()
}

/// 钳位到 (0,0)-(w,h)。
pub fn clamp_point(p: Point, w: f64, h: f64) -> Point {
    Point {
        x: p.x.clamp(0.0, w),
        y: p.y.clamp(0.0, h),
    }
}

/// 选点：过滤面积<0.99 的退化 quad，取首个有效 quad 的中点。
/// 全退化→NotVisible；有有效 quad 但中点全在视口外→NotInViewport。
pub fn pick_click_point(quads: &[Quad], vw: f64, vh: f64) -> Result<Point, GeomError> {
    let mut any_valid = false;
    for q in quads {
        let Some(pts) = quad_to_points(q) else {
            continue;
        };
        if shoelace_area(&pts) <= 0.99 {
            continue;
        }
        any_valid = true;
        let center = Point {
            x: (pts[0].x + pts[1].x + pts[2].x + pts[3].x) / 4.0,
            y: (pts[0].y + pts[1].y + pts[2].y + pts[3].y) / 4.0,
        };
        // 中点在视口内→采纳（钳位防边界外溢）。
        if center.x >= 0.0 && center.x <= vw && center.y >= 0.0 && center.y <= vh {
            return Ok(clamp_point(center, vw, vh));
        }
    }
    if any_valid {
        Err(GeomError::NotInViewport)
    } else {
        Err(GeomError::NotVisible)
    }
}

/// iframe 链偏移累加（主帧 (0,0) + 各层左上角累加）。
pub fn frame_offset(chain: &[FrameRect]) -> Point {
    let mut p = Point { x: 0.0, y: 0.0 };
    for r in chain {
        p.x += r.x;
        p.y += r.y;
    }
    p
}

// ═══════════════════════════════════════════════════════════════════════════
// CDP 输入合成（B5，DESIGN §10 设计裁决④）：getContentQuads 取几何 → dispatchMouseEvent /
// dispatchKeyEvent / insertText。**全程禁 DPR**（x/y 原样 CSS 像素）。所有 send 经
// map_transport_err，绝不 panic。
// ═══════════════════════════════════════════════════════════════════════════

/// CDP modifiers 位掩码约定（dispatchMouseEvent / dispatchKeyEvent 的 `modifiers` 字段）：
/// Alt=1, Ctrl=2, Meta/Command=4, Shift=8（Input.pdl 注释）。
pub mod modifier_bits {
    pub const ALT: u32 = 1;
    pub const CTRL: u32 = 2;
    pub const META: u32 = 4;
    pub const SHIFT: u32 = 8;
}

/// 一个解析好的键击：修饰位掩码 + 主键的 DOM `key` / `code` / Windows 虚拟键码。
///
/// `key`：active-modifier 下的 DOM key 值（如 `"a"` / `"Enter"` / `"Tab"`；US 布局，**不**含 shift
/// 大写——shift 由 `modifiers` 表达，PW 同此约定，让页面 `keydown` 收到正确 `e.shiftKey` 而非
/// 直接收到大写 key）。`code`：物理键 DOM code（如 `"KeyA"` / `"Enter"` / `"Tab"`）。
/// `vk`：Windows 虚拟键码（dispatchKeyEvent `windowsVirtualKeyCode`；快捷键路由要它，否则
/// `Ctrl+A` 在 Windows Chromium 上不触发全选）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyChord {
    /// CDP modifiers 位掩码（[`modifier_bits`] 的或）。
    pub modifiers: u32,
    /// 主键 DOM `key` 值（US 布局，无 shift 大写转换）。
    pub key: String,
    /// 主键物理 DOM `code`（如 `"KeyA"`）。
    pub code: String,
    /// 主键 Windows 虚拟键码（`windowsVirtualKeyCode`）。
    pub vk: i64,
}

/// 解析组合键时的错误（纯逻辑；不进浏览器）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyComboError {
    /// 空串 / 全是修饰键无主键 / 出现两个主键。
    Malformed(String),
    /// 主键 token 不在 US 布局已知映射表里。
    UnknownKey(String),
}

impl std::fmt::Display for KeyComboError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyComboError::Malformed(s) => write!(f, "malformed key combo: {s}"),
            KeyComboError::UnknownKey(k) => write!(f, "unknown key token: {k}"),
        }
    }
}

/// 把一个 token（不区分大小写）识别为修饰键的位值；非修饰键返回 `None`。
fn modifier_bit_for(token: &str) -> Option<u32> {
    match token.to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Some(modifier_bits::CTRL),
        "alt" | "option" => Some(modifier_bits::ALT),
        "shift" => Some(modifier_bits::SHIFT),
        "meta" | "cmd" | "command" | "super" | "win" => Some(modifier_bits::META),
        // 跨平台加速键（Playwright `ControlOrMeta` 同款）：mac→Meta(Cmd),其它→Control。让 agent/
        // 提示词用 `ControlOrMeta+A` 等就在 mac 发 Meta（真全选 + 绕开裸 Ctrl+A 的 mac headless 挂起）。
        "controlormeta" | "ctrlormeta" | "cmdorctrl" => Some(if cfg!(target_os = "macos") {
            modifier_bits::META
        } else {
            modifier_bits::CTRL
        }),
        _ => None,
    }
}

/// 把一个主键 token（US 布局）映射成 `(key, code, vk)`。识别不了返回 `None`。
///
/// 覆盖：单个 ASCII 字母 / 数字 + 常用具名键（Enter/Tab/Escape/方向键/Backspace/Delete/Space/
/// Home/End/PageUp/PageDown）。文本输入不走这里（走 insert_text）；这里只服务**快捷键 / 导航键**。
fn map_main_key(token: &str) -> Option<(String, String, i64)> {
    // 单个 ASCII 字母：key 用小写（shift 由 modifiers 表达），code = "Key<大写>"，vk = 大写 ASCII。
    if token.len() == 1
        && let Some(c) = token.chars().next()
    {
        if c.is_ascii_alphabetic() {
            let lower = c.to_ascii_lowercase();
            let upper = c.to_ascii_uppercase();
            return Some((lower.to_string(), format!("Key{upper}"), upper as i64));
        }
        if c.is_ascii_digit() {
            // 数字行（非 keypad）：code = "Digit<n>"，vk = ASCII 码（'0'..'9' = 48..57）。
            return Some((c.to_string(), format!("Digit{c}"), c as i64));
        }
    }
    // 具名键（不区分大小写匹配，规范化成 DOM 标准 key 名）。vk 取 Windows 虚拟键码。
    let (key, code, vk) = match token.to_ascii_lowercase().as_str() {
        "enter" | "return" => ("Enter", "Enter", 13),
        "tab" => ("Tab", "Tab", 9),
        "escape" | "esc" => ("Escape", "Escape", 27),
        "backspace" => ("Backspace", "Backspace", 8),
        "delete" | "del" => ("Delete", "Delete", 46),
        "space" | "spacebar" => (" ", "Space", 32),
        "arrowup" | "up" => ("ArrowUp", "ArrowUp", 38),
        "arrowdown" | "down" => ("ArrowDown", "ArrowDown", 40),
        "arrowleft" | "left" => ("ArrowLeft", "ArrowLeft", 37),
        "arrowright" | "right" => ("ArrowRight", "ArrowRight", 39),
        "home" => ("Home", "Home", 36),
        "end" => ("End", "End", 35),
        "pageup" | "pgup" => ("PageUp", "PageUp", 33),
        "pagedown" | "pgdn" => ("PageDown", "PageDown", 34),
        _ => return None,
    };
    Some((key.to_string(), code.to_string(), vk))
}

/// **[纯逻辑] 解析组合键字符串**（`"Ctrl+A"` / `"Enter"` / `"Shift+Tab"`…）成 [`KeyChord`]。
///
/// 按 `+` 切 token（首尾空白容忍）：修饰键 token 累加进 modifiers 位掩码；**恰好一个**非修饰
/// token 作主键经 [`map_main_key`]（US 布局）解析。规则：
/// - 空串 / 仅修饰键无主键 / 多于一个主键 → [`KeyComboError::Malformed`]；
/// - 主键 token 不在已知映射 → [`KeyComboError::UnknownKey`]。
///
/// 这是 B5 的可单测核心（纯函数，不进浏览器）：[`dispatch_key_combo`] 解析出 [`KeyChord`] 后才
/// 发 CDP keyDown/keyUp。**不**做 shift→大写转换（大写由 modifiers 的 Shift 位表达，与 PW 一致）。
pub fn parse_key_combo(combo: &str) -> Result<KeyChord, KeyComboError> {
    let mut modifiers: u32 = 0;
    let mut main: Option<(String, String, i64)> = None;

    let mut saw_token = false;
    for raw in combo.split('+') {
        let token = raw.trim();
        if token.is_empty() {
            // 空 token（如 "Ctrl+" 或 "++"）：畸形。
            return Err(KeyComboError::Malformed(combo.to_string()));
        }
        saw_token = true;
        if let Some(bit) = modifier_bit_for(token) {
            modifiers |= bit;
            continue;
        }
        // 非修饰 token = 主键。已经有主键则畸形（两个主键）。
        if main.is_some() {
            return Err(KeyComboError::Malformed(combo.to_string()));
        }
        match map_main_key(token) {
            Some(parsed) => main = Some(parsed),
            None => return Err(KeyComboError::UnknownKey(token.to_string())),
        }
    }

    if !saw_token {
        return Err(KeyComboError::Malformed(combo.to_string()));
    }
    let (key, code, vk) = main.ok_or_else(|| KeyComboError::Malformed(combo.to_string()))?;
    Ok(KeyChord {
        modifiers,
        key,
        code,
        vk,
    })
}

/// 把 [`KeyComboError`] 映射成 [`BrowserError`]：组合键文法/未知键都是调用方传错（非瞬态），
/// 归 `Other`（带原文供诊断）。
fn map_key_combo_err(e: KeyComboError) -> BrowserError {
    BrowserError::Other(format!("key combo parse error: {e}"))
}

/// 取元素的 content quads：发 `DOM.getContentQuads{objectId}`，返回扁平 8 数 quad 列表
/// （CSS 像素，**原样**给 [`pick_click_point`]，零 DPR 换算）。
///
/// 元素不可见 / 无布局盒（display:none 等）→ getContentQuads 返空 `quads`（或不带字段）→ 这里返
/// 空 `Vec`（调用方据 [`pick_click_point`] 报 `NotVisible`，语义统一）。
pub async fn get_content_quads(
    conn: &Connection,
    session: &str,
    object_id: &str,
) -> Result<Vec<Quad>, BrowserError> {
    let params = GetContentQuadsParams::builder()
        .object_id(RemoteObjectId::new(object_id.to_string()))
        .build();
    let result = conn
        .send::<GetContentQuadsParams>(session, &params)
        .await
        .map_err(map_transport_err)?;
    // 直接从 raw JSON 取 `quads`（Vec<Vec<f64>>），与本模块 `Quad = Vec<f64>` 对齐；元素无盒 → 缺
    // 字段 / 空数组都归空列表（不报错，交 pick_click_point 判 NotVisible）。
    let quads = result
        .get("quads")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|q| {
                    q.as_array().map(|nums| {
                        nums.iter().filter_map(serde_json::Value::as_f64).collect::<Vec<f64>>()
                    })
                })
                .collect::<Vec<Quad>>()
        })
        .unwrap_or_default();
    Ok(quads)
}

/// 取主框架 viewport 的 CSS 像素尺寸 `(innerWidth, innerHeight)`：一次
/// `Runtime.evaluate("[window.innerWidth, window.innerHeight]", returnByValue)`。**禁 DPR**：
/// innerWidth/innerHeight 本就是 CSS 像素，[`pick_click_point`] 用它判中点是否在视口内（同坐标系）。
pub async fn viewport_size(conn: &Connection, session: &str) -> Result<(f64, f64), BrowserError> {
    let mut params = EvaluateParams::new("[window.innerWidth, window.innerHeight]".to_string());
    params.return_by_value = Some(true);
    let result = conn
        .send::<EvaluateParams>(session, &params)
        .await
        .map_err(map_transport_err)?;
    if let Some(ex) = result.get("exceptionDetails") {
        return Err(BrowserError::Other(format!("viewport_size eval threw: {ex}")));
    }
    let arr = result
        .get("result")
        .and_then(|r| r.get("value"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            BrowserError::Other(format!("viewport_size: unexpected eval result shape: {result}"))
        })?;
    let w = arr.first().and_then(serde_json::Value::as_f64);
    let h = arr.get(1).and_then(serde_json::Value::as_f64);
    match (w, h) {
        (Some(w), Some(h)) => Ok((w, h)),
        _ => Err(BrowserError::Other(format!(
            "viewport_size: innerWidth/innerHeight not numbers: {arr:?}"
        ))),
    }
}

/// 发一条 `Input.dispatchMouseEvent`（内部）。`point` 直接当 CSS 像素 x/y——**零 DPR**：无任何
/// deviceScaleFactor 乘除（这是 B5 的铁律，见模块 doc）。
async fn dispatch_mouse(
    conn: &Connection,
    session: &str,
    r#type: DispatchMouseEventType,
    point: Point,
    button: Option<MouseButton>,
    click_count: Option<i64>,
) -> Result<(), BrowserError> {
    let mut params = DispatchMouseEventParams::new(r#type, point.x, point.y);
    params.button = button;
    params.click_count = click_count;
    conn.send::<DispatchMouseEventParams>(session, &params)
        .await
        .map_err(map_transport_err)?;
    Ok(())
}

/// 在 `point`（CSS 像素，**零 DPR**）左键单击：`mousePressed` + `mouseReleased`（button=left,
/// clickCount=1）。`point` 来自 [`get_content_quads`] → [`pick_click_point`]，原样下发。
pub async fn dispatch_click(
    conn: &Connection,
    session: &str,
    point: Point,
) -> Result<(), BrowserError> {
    dispatch_mouse(
        conn,
        session,
        DispatchMouseEventType::MousePressed,
        point,
        Some(MouseButton::Left),
        Some(1),
    )
    .await?;
    dispatch_mouse(
        conn,
        session,
        DispatchMouseEventType::MouseReleased,
        point,
        Some(MouseButton::Left),
        Some(1),
    )
    .await?;
    Ok(())
}

/// 把鼠标移到 `point`（CSS 像素，**零 DPR**）：`mouseMoved`（hover 用，无按键）。
pub async fn dispatch_mouse_move(
    conn: &Connection,
    session: &str,
    point: Point,
) -> Result<(), BrowserError> {
    dispatch_mouse(
        conn,
        session,
        DispatchMouseEventType::MouseMoved,
        point,
        None,
        None,
    )
    .await
}

/// 发一条 `Input.dispatchKeyEvent`（内部）。`text` 仅 keyDown 时按需带（具名键 / 快捷键不带 text，
/// 普通字符 keyDown 才带，让页面收到 `input`/`keypress`）。
async fn dispatch_key(
    conn: &Connection,
    session: &str,
    r#type: DispatchKeyEventType,
    chord: &KeyChord,
    text: Option<&str>,
) -> Result<(), BrowserError> {
    #[cfg(target_os = "macos")]
    let is_key_down = matches!(r#type, DispatchKeyEventType::KeyDown);
    let mut params = DispatchKeyEventParams::new(r#type);
    params.modifiers = Some(i64::from(chord.modifiers));
    params.key = Some(chord.key.clone());
    params.code = Some(chord.code.clone());
    params.windows_virtual_key_code = Some(chord.vk);
    params.native_virtual_key_code = Some(chord.vk);
    params.text = text.map(str::to_string);
    // macOS：合成键经 CDP **不带 `commands` 时 Blink 不跑 Cocoa 编辑命令**（Cmd+A 不全选、Cmd+C 不复制、
    // 方向键不按编辑语义移动…）。据 [`mac_editing_commands`] 补 `commands`（仅 keyDown）——让 mac 键盘
    // 编辑/导航快捷键经 CDP 真生效（Playwright crInput.ts 同款）。非 mac 靠 windowsVirtualKeyCode 路由。
    #[cfg(target_os = "macos")]
    if is_key_down {
        let cmds = mac_editing_commands(&chord.code, chord.modifiers);
        if !cmds.is_empty() {
            params.commands = Some(cmds);
        }
    }
    conn.send::<DispatchKeyEventParams>(session, &params)
        .await
        .map_err(map_transport_err)?;
    Ok(())
}

/// **macEditingCommands（mac 专用）**：`(code, modifiers)` → CDP `Input.dispatchKeyEvent.commands`。
/// 移植 Playwright `packages/playwright-core/src/server/macEditingCommands.ts`（Apache-2.0,署名
/// Microsoft / Google）。与 `crInput.ts::_commandsForCode` 同：shortcut = 活跃修饰键（Shift, Control,
/// Alt, Meta 顺序）+ DOM `code`,join `+`;查表后**过滤 `insert*`**（那些由 [`insert_text`] 文本路径
/// 处理）+ **去尾冒号**（CDP 要 `selectAll` 而非 `selectAll:`）。仅 keyDown 带。
#[cfg(target_os = "macos")]
fn mac_editing_commands(code: &str, modifiers: u32) -> Vec<String> {
    let mut parts: Vec<&str> = Vec::new();
    if modifiers & modifier_bits::SHIFT != 0 {
        parts.push("Shift");
    }
    if modifiers & modifier_bits::CTRL != 0 {
        parts.push("Control");
    }
    if modifiers & modifier_bits::ALT != 0 {
        parts.push("Alt");
    }
    if modifiers & modifier_bits::META != 0 {
        parts.push("Meta");
    }
    parts.push(code);
    let shortcut = parts.join("+");
    mac_editing_raw(&shortcut)
        .iter()
        .filter(|c| !c.starts_with("insert"))
        .map(|c| c.trim_end_matches(':').to_string())
        .collect()
}

/// macEditingCommands 原始表（带尾冒号；逐行对应 macEditingCommands.ts,便于升级 diff 人审）。
#[cfg(target_os = "macos")]
fn mac_editing_raw(shortcut: &str) -> &'static [&'static str] {
    match shortcut {
        "Backspace" => &["deleteBackward:"],
        "Enter" => &["insertNewline:"],
        "NumpadEnter" => &["insertNewline:"],
        "Escape" => &["cancelOperation:"],
        "ArrowUp" => &["moveUp:"],
        "ArrowDown" => &["moveDown:"],
        "ArrowLeft" => &["moveLeft:"],
        "ArrowRight" => &["moveRight:"],
        "F5" => &["complete:"],
        "Delete" => &["deleteForward:"],
        "Home" => &["scrollToBeginningOfDocument:"],
        "End" => &["scrollToEndOfDocument:"],
        "PageUp" => &["scrollPageUp:"],
        "PageDown" => &["scrollPageDown:"],
        "Shift+Backspace" => &["deleteBackward:"],
        "Shift+Enter" => &["insertNewline:"],
        "Shift+NumpadEnter" => &["insertNewline:"],
        "Shift+Escape" => &["cancelOperation:"],
        "Shift+ArrowUp" => &["moveUpAndModifySelection:"],
        "Shift+ArrowDown" => &["moveDownAndModifySelection:"],
        "Shift+ArrowLeft" => &["moveLeftAndModifySelection:"],
        "Shift+ArrowRight" => &["moveRightAndModifySelection:"],
        "Shift+F5" => &["complete:"],
        "Shift+Delete" => &["deleteForward:"],
        "Shift+Home" => &["moveToBeginningOfDocumentAndModifySelection:"],
        "Shift+End" => &["moveToEndOfDocumentAndModifySelection:"],
        "Shift+PageUp" => &["pageUpAndModifySelection:"],
        "Shift+PageDown" => &["pageDownAndModifySelection:"],
        "Shift+Numpad5" => &["delete:"],
        "Control+Tab" => &["selectNextKeyView:"],
        "Control+Enter" => &["insertLineBreak:"],
        "Control+NumpadEnter" => &["insertLineBreak:"],
        "Control+Quote" => &["insertSingleQuoteIgnoringSubstitution:"],
        "Control+KeyA" => &["moveToBeginningOfParagraph:"],
        "Control+KeyB" => &["moveBackward:"],
        "Control+KeyD" => &["deleteForward:"],
        "Control+KeyE" => &["moveToEndOfParagraph:"],
        "Control+KeyF" => &["moveForward:"],
        "Control+KeyH" => &["deleteBackward:"],
        "Control+KeyK" => &["deleteToEndOfParagraph:"],
        "Control+KeyL" => &["centerSelectionInVisibleArea:"],
        "Control+KeyN" => &["moveDown:"],
        "Control+KeyO" => &["insertNewlineIgnoringFieldEditor:", "moveBackward:"],
        "Control+KeyP" => &["moveUp:"],
        "Control+KeyT" => &["transpose:"],
        "Control+KeyV" => &["pageDown:"],
        "Control+KeyY" => &["yank:"],
        "Control+Backspace" => &["deleteBackwardByDecomposingPreviousCharacter:"],
        "Control+ArrowUp" => &["scrollPageUp:"],
        "Control+ArrowDown" => &["scrollPageDown:"],
        "Control+ArrowLeft" => &["moveToLeftEndOfLine:"],
        "Control+ArrowRight" => &["moveToRightEndOfLine:"],
        "Shift+Control+Enter" => &["insertLineBreak:"],
        "Shift+Control+NumpadEnter" => &["insertLineBreak:"],
        "Shift+Control+Tab" => &["selectPreviousKeyView:"],
        "Shift+Control+Quote" => &["insertDoubleQuoteIgnoringSubstitution:"],
        "Shift+Control+KeyA" => &["moveToBeginningOfParagraphAndModifySelection:"],
        "Shift+Control+KeyB" => &["moveBackwardAndModifySelection:"],
        "Shift+Control+KeyE" => &["moveToEndOfParagraphAndModifySelection:"],
        "Shift+Control+KeyF" => &["moveForwardAndModifySelection:"],
        "Shift+Control+KeyN" => &["moveDownAndModifySelection:"],
        "Shift+Control+KeyP" => &["moveUpAndModifySelection:"],
        "Shift+Control+KeyV" => &["pageDownAndModifySelection:"],
        "Shift+Control+Backspace" => &["deleteBackwardByDecomposingPreviousCharacter:"],
        "Shift+Control+ArrowUp" => &["scrollPageUp:"],
        "Shift+Control+ArrowDown" => &["scrollPageDown:"],
        "Shift+Control+ArrowLeft" => &["moveToLeftEndOfLineAndModifySelection:"],
        "Shift+Control+ArrowRight" => &["moveToRightEndOfLineAndModifySelection:"],
        "Alt+Backspace" => &["deleteWordBackward:"],
        "Alt+Enter" => &["insertNewlineIgnoringFieldEditor:"],
        "Alt+NumpadEnter" => &["insertNewlineIgnoringFieldEditor:"],
        "Alt+Escape" => &["complete:"],
        "Alt+ArrowUp" => &["moveBackward:", "moveToBeginningOfParagraph:"],
        "Alt+ArrowDown" => &["moveForward:", "moveToEndOfParagraph:"],
        "Alt+ArrowLeft" => &["moveWordLeft:"],
        "Alt+ArrowRight" => &["moveWordRight:"],
        "Alt+Delete" => &["deleteWordForward:"],
        "Alt+PageUp" => &["pageUp:"],
        "Alt+PageDown" => &["pageDown:"],
        "Shift+Alt+Backspace" => &["deleteWordBackward:"],
        "Shift+Alt+Enter" => &["insertNewlineIgnoringFieldEditor:"],
        "Shift+Alt+NumpadEnter" => &["insertNewlineIgnoringFieldEditor:"],
        "Shift+Alt+Escape" => &["complete:"],
        "Shift+Alt+ArrowUp" => &["moveParagraphBackwardAndModifySelection:"],
        "Shift+Alt+ArrowDown" => &["moveParagraphForwardAndModifySelection:"],
        "Shift+Alt+ArrowLeft" => &["moveWordLeftAndModifySelection:"],
        "Shift+Alt+ArrowRight" => &["moveWordRightAndModifySelection:"],
        "Shift+Alt+Delete" => &["deleteWordForward:"],
        "Shift+Alt+PageUp" => &["pageUp:"],
        "Shift+Alt+PageDown" => &["pageDown:"],
        "Control+Alt+KeyB" => &["moveWordBackward:"],
        "Control+Alt+KeyF" => &["moveWordForward:"],
        "Control+Alt+Backspace" => &["deleteWordBackward:"],
        "Shift+Control+Alt+KeyB" => &["moveWordBackwardAndModifySelection:"],
        "Shift+Control+Alt+KeyF" => &["moveWordForwardAndModifySelection:"],
        "Shift+Control+Alt+Backspace" => &["deleteWordBackward:"],
        "Meta+NumpadSubtract" => &["cancel:"],
        "Meta+Backspace" => &["deleteToBeginningOfLine:"],
        "Meta+ArrowUp" => &["moveToBeginningOfDocument:"],
        "Meta+ArrowDown" => &["moveToEndOfDocument:"],
        "Meta+ArrowLeft" => &["moveToLeftEndOfLine:"],
        "Meta+ArrowRight" => &["moveToRightEndOfLine:"],
        "Shift+Meta+NumpadSubtract" => &["cancel:"],
        "Shift+Meta+Backspace" => &["deleteToBeginningOfLine:"],
        "Shift+Meta+ArrowUp" => &["moveToBeginningOfDocumentAndModifySelection:"],
        "Shift+Meta+ArrowDown" => &["moveToEndOfDocumentAndModifySelection:"],
        "Shift+Meta+ArrowLeft" => &["moveToLeftEndOfLineAndModifySelection:"],
        "Shift+Meta+ArrowRight" => &["moveToRightEndOfLineAndModifySelection:"],
        "Meta+KeyA" => &["selectAll:"],
        "Meta+KeyC" => &["copy:"],
        "Meta+KeyX" => &["cut:"],
        "Meta+KeyV" => &["paste:"],
        "Meta+KeyZ" => &["undo:"],
        "Shift+Meta+KeyZ" => &["redo:"],
        _ => &[],
    }
}

/// 修饰位 → 该修饰键的 `(key, code, vk)`（按下/释放修饰键自身的 keyDown/keyUp 用）。
fn modifier_chord(bit: u32) -> KeyChord {
    let (key, code, vk) = match bit {
        b if b == modifier_bits::CTRL => ("Control", "ControlLeft", 17),
        b if b == modifier_bits::ALT => ("Alt", "AltLeft", 18),
        b if b == modifier_bits::SHIFT => ("Shift", "ShiftLeft", 16),
        b if b == modifier_bits::META => ("Meta", "MetaLeft", 91),
        _ => ("Unidentified", "", 0),
    };
    KeyChord {
        modifiers: 0,
        key: key.to_string(),
        code: code.to_string(),
        vk,
    }
}

/// **主键 keyDown 该带的 `text`**（CDP `Input.dispatchKeyEvent{text}`）。合成 keyDown 不带 text 时
/// Chromium **不跑该键的默认动作**（Enter 不隐式提交表单 / 不在 textarea 换行；可打印字符不产生 `input`）。
/// 故对**会产生文本/默认动作**的键，keyDown 要带 text：
/// - `Enter` → `"\r"`（隐式表单提交 / textarea 换行的默认动作 token）；
/// - 单个可打印字符（key 长度 1，如 `"a"`/`"5"`/`" "`）→ 该字符（Shift 字母大写）——让 tier3 逐字符
///   逃生（[`crate::backend::cdp::CdpBackend::act_type_per_char`]）的 keyDown 真产生 `input`；
/// - 其它具名键（Tab/Escape/方向键/…）→ `None`（无文本默认动作）。
///
/// **带 Ctrl/Alt/Meta 修饰时一律 None**：那是快捷键（`Ctrl+A`），不应产生字符/默认动作。
fn chord_text(chord: &KeyChord) -> Option<String> {
    // Ctrl/Alt/Meta 修饰 → 快捷键，无文本默认动作。
    let cmd_like =
        chord.modifiers & (modifier_bits::CTRL | modifier_bits::ALT | modifier_bits::META) != 0;
    if cmd_like {
        return None;
    }
    if chord.key == "Enter" {
        return Some("\r".to_string());
    }
    // 单个可打印字符：返该字符（Shift 位 → 大写）。key 已是小写（shift 由 modifiers 表达），故这里据
    // Shift 位还原大小写。多字符具名键（"ArrowUp"/"Tab"…）不匹配 → None。
    let mut chars = chord.key.chars();
    if let (Some(c), None) = (chars.next(), chars.clone().next())
        && (c.is_ascii_graphic() || c == ' ')
    {
        let shifted = chord.modifiers & modifier_bits::SHIFT != 0;
        let out = if shifted {
            c.to_ascii_uppercase()
        } else {
            c
        };
        return Some(out.to_string());
    }
    None
}


/// **按下**（各发一条 keyDown，modifiers 渐累）→ 主键 keyDown + keyUp（带全部 modifiers）→ 修饰键
/// 逆序**释放**（keyUp）。**失败时释放已按下的修饰键**（不留卡住的修饰态）。
///
/// 主键不带 `text`（这是快捷键 / 导航键路径；文本输入走 [`insert_text`]）。**零 DPR**（键盘路径
/// 本无坐标）。所有 send 经 `map_transport_err`，绝不 panic。
pub async fn dispatch_key_combo(
    conn: &Connection,
    session: &str,
    keys: &str,
) -> Result<(), BrowserError> {
    let chord = parse_key_combo(keys).map_err(map_key_combo_err)?;

    // 修饰键按固定顺序按下（Ctrl, Alt, Shift, Meta），渐累 modifiers——让每条修饰 keyDown 的
    // modifiers 反映「此刻已按下的修饰键」。记录已按下的位，用于失败 / 收尾时逆序释放。
    let order = [
        modifier_bits::CTRL,
        modifier_bits::ALT,
        modifier_bits::SHIFT,
        modifier_bits::META,
    ];
    let mut pressed: Vec<u32> = Vec::new();
    let mut accumulated: u32 = 0;

    // 失败时释放已按下的修饰键（逆序 keyUp）。best-effort：释放本身失败也不掩盖原错。
    async fn release_pressed(conn: &Connection, session: &str, pressed: &[u32], acc: u32) {
        let mut acc = acc;
        for &bit in pressed.iter().rev() {
            acc &= !bit;
            let mut mc = modifier_chord(bit);
            mc.modifiers = acc;
            let _ = dispatch_key(conn, session, DispatchKeyEventType::KeyUp, &mc, None).await;
        }
    }

    for &bit in &order {
        if chord.modifiers & bit == 0 {
            continue;
        }
        accumulated |= bit;
        let mut mc = modifier_chord(bit);
        mc.modifiers = accumulated;
        if let Err(e) = dispatch_key(conn, session, DispatchKeyEventType::KeyDown, &mc, None).await {
            release_pressed(conn, session, &pressed, accumulated & !bit).await;
            return Err(e);
        }
        pressed.push(bit);
    }

    // 主键 keyDown（带全部 modifiers）→ keyUp。任一失败都先释放修饰键再返错。
    let main_down = {
        let mut k = chord.clone();
        k.modifiers = accumulated;
        k
    };
    // 主键 keyDown 的 `text`：Enter→"\r"（**触发浏览器隐式表单提交 / textarea 换行的默认动作**——
    // 合成 keyDown 不带 text 时 Chromium 不跑 Enter 的默认动作）；其它键不带 text（导航/快捷键路径，
    // 文本输入走 insert_text）。无 Ctrl/Alt/Meta 修饰时才带（带这些修饰是快捷键，不应产生字符/默认动作）。
    let main_text = chord_text(&main_down);
    if let Err(e) = dispatch_key(
        conn,
        session,
        DispatchKeyEventType::KeyDown,
        &main_down,
        main_text.as_deref(),
    )
    .await
    {
        release_pressed(conn, session, &pressed, accumulated).await;
        return Err(e);
    }
    if let Err(e) = dispatch_key(conn, session, DispatchKeyEventType::KeyUp, &main_down, None).await {
        release_pressed(conn, session, &pressed, accumulated).await;
        return Err(e);
    }

    // 正常收尾：逆序释放修饰键（modifiers 渐减）。这里失败要传播（修饰态没干净释放是真问题）。
    let mut acc = accumulated;
    for &bit in pressed.iter().rev() {
        acc &= !bit;
        let mut mc = modifier_chord(bit);
        mc.modifiers = acc;
        dispatch_key(conn, session, DispatchKeyEventType::KeyUp, &mc, None).await?;
    }
    Ok(())
}

/// **插入文本**（`Input.insertText{text}`）：IME / secret / fill 的「needs input」兜底——不经
/// 单键合成，直接让浏览器把整段 `text` 当输入法提交插入聚焦元素。**零 DPR**（无坐标）。
pub async fn insert_text(conn: &Connection, session: &str, text: &str) -> Result<(), BrowserError> {
    let params = InsertTextParams::new(text.to_string());
    conn.send::<InsertTextParams>(session, &params)
        .await
        .map_err(map_transport_err)?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// 滚动（C2，DESIGN §9/§11）：视口滚动经 `Input.dispatchMouseEvent{mouseWheel, deltaX/deltaY}`
// （CSS 像素 delta，**零 DPR**）。方向 + 可选量 → (deltaX, deltaY) 由纯函数 [`scroll_deltas`]
// 换算（可单测）。element-target 滚动走注入 scrollIntoView（在 injected.rs / actions.rs）。
// ═══════════════════════════════════════════════════════════════════════════

use crate::actions::ScrollDir;

/// 视口滚动一「步」的默认 CSS 像素量（`amount` 为 None 时用它；约一屏的多半，对齐常见 wheel notch
/// 的几倍——足够推进又不过冲）。
pub const DEFAULT_SCROLL_STEP: f64 = 400.0;

/// **[纯逻辑] 方向 + 可选量 → mouseWheel 的 `(deltaX, deltaY)`**（CSS 像素，**零 DPR**）。
///
/// CDP `Input.dispatchMouseEvent{type:mouseWheel}` 的 `deltaY > 0` = 内容向**上**移动（即视口向**下**
/// 滚），`deltaX > 0` = 向**右**滚（与 `window.scrollBy` 符号一致）。故：
/// - [`ScrollDir::Down`] → `(0, +amount)`；[`ScrollDir::Up`] → `(0, -amount)`；
/// - [`ScrollDir::Right`] → `(+amount, 0)`；[`ScrollDir::Left`] → `(-amount, 0)`。
///
/// `amount` 为 `None` → 用 [`DEFAULT_SCROLL_STEP`]。负 `amount` 取绝对值（方向由 `dir` 决定，量是标量）。
pub fn scroll_deltas(dir: ScrollDir, amount: Option<f64>) -> (f64, f64) {
    let step = amount.map(f64::abs).unwrap_or(DEFAULT_SCROLL_STEP);
    match dir {
        ScrollDir::Down => (0.0, step),
        ScrollDir::Up => (0.0, -step),
        ScrollDir::Right => (step, 0.0),
        ScrollDir::Left => (-step, 0.0),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CdpBackend 的输入合成方法：把上面的自由函数接到主 page session。把「选哪个 session」收口在
// backend（B5 范围只覆盖主 page session 的元素；OOPIF 跨 session 留 C1，届时 ObjectHandle 需带帧
// 路由）。C1 act facade 经这些方法串 hit-target + 重试。**零 DPR**（坐标原样 CSS 像素）。
// ═══════════════════════════════════════════════════════════════════════════

use crate::actionability::ObjectHandle;
use crate::backend::cdp::CdpBackend;

impl CdpBackend {
    /// 取已反查元素句柄的 content quads（CSS 像素，**零 DPR**），喂 [`pick_click_point`] 选点。
    /// 元素无布局盒（不可见）→ 空列表（调用方 [`pick_click_point`] 报 `NotVisible`）。
    /// **D1**：page session 经 active tab 解引用（async 取，立即释放 tabs 锁）。
    pub async fn element_content_quads(&self, h: &ObjectHandle) -> Result<Vec<Quad>, BrowserError> {
        let session = self.page_session_id().await?;
        get_content_quads(self.conn(), &session, &h.object_id).await
    }

    /// 主框架 viewport CSS 像素尺寸 `(innerW, innerH)`（**零 DPR**），喂 [`pick_click_point`] 判视口内。
    pub async fn viewport_size(&self) -> Result<(f64, f64), BrowserError> {
        let session = self.page_session_id().await?;
        viewport_size(self.conn(), &session).await
    }

    /// 在 `point`（CSS 像素，**零 DPR**）左键单击（mousePressed + mouseReleased）。
    pub async fn click_at(&self, point: Point) -> Result<(), BrowserError> {
        let session = self.page_session_id().await?;
        dispatch_click(self.conn(), &session, point).await
    }

    /// 把鼠标移到 `point`（CSS 像素，**零 DPR**）（hover；mouseMoved）。
    pub async fn mouse_move_to(&self, point: Point) -> Result<(), BrowserError> {
        let session = self.page_session_id().await?;
        dispatch_mouse_move(self.conn(), &session, point).await
    }

    /// 合成组合键（US 布局）：`"Ctrl+A"` / `"Enter"` / `"Shift+Tab"`…（见 [`dispatch_key_combo`]）。
    pub async fn key_combo(&self, keys: &str) -> Result<(), BrowserError> {
        let session = self.page_session_id().await?;
        dispatch_key_combo(self.conn(), &session, keys).await
    }

    /// 插入文本（`Input.insertText`）：IME / secret / fill 兜底（见 [`insert_text`]）。
    pub async fn type_text(&self, text: &str) -> Result<(), BrowserError> {
        let session = self.page_session_id().await?;
        insert_text(self.conn(), &session, text).await
    }

    /// **视口滚动**（C2）：把视口按 `(delta_x, delta_y)`（CSS 像素 **零 DPR**，[`scroll_deltas`] 按
    /// direction/amount 换算）滚动。**注入 `window.scrollBy`**（而非 `Input.dispatchMouseEvent{mouseWheel}`）：
    /// DESIGN §9 两者皆可，但合成 wheel 事件在 headless 下常不驱动滚动（落点/可滚动容器命中弱），
    /// `scrollBy` 确定性强、跨 headless/headful 一致，且 `behavior:'instant'` 同步落定便于 verify 读回。
    /// `delta_y>0` = 视口下滚（与 [`scroll_deltas`] 符号约定一致）。**禁 DPR**（scrollBy 是 CSS 像素语义）。
    pub async fn scroll_viewport(&self, delta_x: f64, delta_y: f64) -> Result<(), BrowserError> {
        let session = self.page_session_id().await?;
        let expr = format!(
            "window.scrollBy({{ left: {delta_x}, top: {delta_y}, behavior: 'instant' }})"
        );
        let mut params = EvaluateParams::new(expr);
        params.return_by_value = Some(true);
        self.conn()
            .send::<EvaluateParams>(&session, &params)
            .await
            .map_err(map_transport_err)?;
        Ok(())
    }

    /// 读当前视口滚动位置 `(scrollX, scrollY)`（verify 锚点：scroll 前后对比证 changed）。
    /// best-effort：读不到 / active tab 缺失返 `(0.0, 0.0)`（不致命；锚点缺失只影响 changed 判定保守）。
    pub async fn scroll_position(&self) -> (f64, f64) {
        let Ok(session) = self.page_session_id().await else {
            return (0.0, 0.0);
        };
        let mut params = EvaluateParams::new("[window.scrollX, window.scrollY]".to_string());
        params.return_by_value = Some(true);
        let Ok(result) = self
            .conn()
            .send::<EvaluateParams>(&session, &params)
            .await
        else {
            return (0.0, 0.0);
        };
        let arr = result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(|v| v.as_array());
        match arr {
            Some(a) => (
                a.first().and_then(serde_json::Value::as_f64).unwrap_or(0.0),
                a.get(1).and_then(serde_json::Value::as_f64).unwrap_or(0.0),
            ),
            None => (0.0, 0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quad_to_points_splits_8_into_4() {
        // CDP quad 是扁平 8 数: [x1,y1,x2,y2,x3,y3,x4,y4]
        let q: Quad = vec![0.0, 0.0, 10.0, 0.0, 10.0, 10.0, 0.0, 10.0];
        let p = quad_to_points(&q).unwrap();
        assert_eq!(p[0], Point { x: 0.0, y: 0.0 });
        assert_eq!(p[2], Point { x: 10.0, y: 10.0 });
    }

    #[test]
    fn shoelace_area_of_unit_square() {
        let p = [
            Point { x: 0.0, y: 0.0 },
            Point { x: 10.0, y: 0.0 },
            Point { x: 10.0, y: 10.0 },
            Point { x: 0.0, y: 10.0 },
        ];
        assert!((shoelace_area(&p) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn pick_click_point_returns_center_of_first_visible_quad() {
        let big: Quad = vec![0.0, 0.0, 20.0, 0.0, 20.0, 20.0, 0.0, 20.0]; // 面积 400
        let pt = pick_click_point(&[big], 100.0, 100.0).unwrap();
        assert_eq!(pt, Point { x: 10.0, y: 10.0 });
    }

    #[test]
    fn pick_click_point_filters_degenerate_and_reports_notvisible() {
        let zero: Quad = vec![5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0]; // 面积 0 (<0.99)
        assert!(matches!(
            pick_click_point(&[zero], 100.0, 100.0),
            Err(GeomError::NotVisible)
        ));
    }

    #[test]
    fn pick_click_point_offscreen_reports_notinviewport() {
        let off: Quad = vec![200.0, 200.0, 220.0, 200.0, 220.0, 220.0, 200.0, 220.0]; // 全在 100x100 视口外
        assert!(matches!(
            pick_click_point(&[off], 100.0, 100.0),
            Err(GeomError::NotInViewport)
        ));
    }

    #[test]
    fn clamp_point_to_viewport() {
        assert_eq!(
            clamp_point(Point { x: 150.0, y: -5.0 }, 100.0, 100.0),
            Point { x: 100.0, y: 0.0 }
        );
    }

    #[test]
    fn frame_offset_accumulates() {
        let chain = vec![FrameRect { x: 10.0, y: 20.0 }, FrameRect { x: 5.0, y: 5.0 }];
        assert_eq!(frame_offset(&chain), Point { x: 15.0, y: 25.0 });
    }

    // ── parse_key_combo（B5 可单测核心，US 布局，CDP 位掩码 Alt=1/Ctrl=2/Meta=4/Shift=8）──

    #[test]
    fn parse_key_combo_ctrl_a() {
        // "Ctrl+A" → modifiers 含 Ctrl 位；主键 key="a"（小写，shift 由 modifiers 表达）/code="KeyA"。
        let c = parse_key_combo("Ctrl+A").unwrap();
        assert_eq!(c.modifiers & modifier_bits::CTRL, modifier_bits::CTRL);
        // 只有 Ctrl，无 shift/alt/meta。
        assert_eq!(c.modifiers, modifier_bits::CTRL);
        assert_eq!(c.key, "a");
        assert_eq!(c.code, "KeyA");
        assert_eq!(c.vk, 'A' as i64); // 65
    }

    #[test]
    fn parse_key_combo_enter() {
        // "Enter" → 无修饰；key/code="Enter"，vk=13。
        let c = parse_key_combo("Enter").unwrap();
        assert_eq!(c.modifiers, 0);
        assert_eq!(c.key, "Enter");
        assert_eq!(c.code, "Enter");
        assert_eq!(c.vk, 13);
    }

    #[test]
    fn parse_key_combo_shift_tab() {
        // "Shift+Tab" → modifiers 含 Shift；key/code="Tab"，vk=9。
        let c = parse_key_combo("Shift+Tab").unwrap();
        assert_eq!(c.modifiers & modifier_bits::SHIFT, modifier_bits::SHIFT);
        assert_eq!(c.modifiers, modifier_bits::SHIFT);
        assert_eq!(c.key, "Tab");
        assert_eq!(c.code, "Tab");
        assert_eq!(c.vk, 9);
    }

    #[test]
    fn parse_key_combo_multiple_modifiers_accumulate() {
        // "Ctrl+Shift+K" → Ctrl|Shift 两位；主键 K。
        let c = parse_key_combo("Ctrl+Shift+K").unwrap();
        assert_eq!(c.modifiers, modifier_bits::CTRL | modifier_bits::SHIFT);
        assert_eq!(c.key, "k");
        assert_eq!(c.code, "KeyK");
    }

    #[test]
    fn parse_key_combo_modifier_aliases() {
        // control/cmd/option/win 等别名归一到正确位。
        assert_eq!(
            parse_key_combo("Control+a").unwrap().modifiers,
            modifier_bits::CTRL
        );
        assert_eq!(
            parse_key_combo("Cmd+a").unwrap().modifiers,
            modifier_bits::META
        );
        assert_eq!(
            parse_key_combo("Option+a").unwrap().modifiers,
            modifier_bits::ALT
        );
        assert_eq!(
            parse_key_combo("Win+a").unwrap().modifiers,
            modifier_bits::META
        );
    }

    #[test]
    fn parse_key_combo_digit_and_named_keys() {
        // 数字键：code="Digit5"，vk=ASCII。
        let c = parse_key_combo("5").unwrap();
        assert_eq!(c.key, "5");
        assert_eq!(c.code, "Digit5");
        assert_eq!(c.vk, '5' as i64);
        // 方向键 / Escape 等具名键。
        assert_eq!(parse_key_combo("ArrowDown").unwrap().code, "ArrowDown");
        assert_eq!(parse_key_combo("Esc").unwrap().key, "Escape");
        assert_eq!(parse_key_combo("Space").unwrap().code, "Space");
    }

    #[test]
    fn parse_key_combo_tolerates_whitespace() {
        // token 首尾空白容忍（"Ctrl + A"）。
        let c = parse_key_combo("Ctrl + A").unwrap();
        assert_eq!(c.modifiers, modifier_bits::CTRL);
        assert_eq!(c.key, "a");
    }

    #[test]
    fn parse_key_combo_rejects_malformed() {
        // 空串 → Malformed。
        assert!(matches!(parse_key_combo(""), Err(KeyComboError::Malformed(_))));
        // 仅修饰键无主键 → Malformed。
        assert!(matches!(
            parse_key_combo("Ctrl"),
            Err(KeyComboError::Malformed(_))
        ));
        assert!(matches!(
            parse_key_combo("Ctrl+Shift"),
            Err(KeyComboError::Malformed(_))
        ));
        // 尾随 + / 空 token → Malformed。
        assert!(matches!(
            parse_key_combo("Ctrl+"),
            Err(KeyComboError::Malformed(_))
        ));
        // 两个主键 → Malformed。
        assert!(matches!(
            parse_key_combo("A+B"),
            Err(KeyComboError::Malformed(_))
        ));
    }

    #[test]
    fn parse_key_combo_rejects_unknown_key() {
        // 不在 US 布局映射表的主键 token → UnknownKey。
        assert!(matches!(
            parse_key_combo("Ctrl+Frobnicate"),
            Err(KeyComboError::UnknownKey(_))
        ));
        // 多字符非具名（不是字母/数字单字符，也不是已知具名键）。
        assert!(matches!(
            parse_key_combo("F13"),
            Err(KeyComboError::UnknownKey(_))
        ));
    }

    #[test]
    fn modifier_chord_maps_each_bit() {
        // 修饰键自身的 keyDown/keyUp 用的 (key, code, vk)。
        assert_eq!(modifier_chord(modifier_bits::CTRL).key, "Control");
        assert_eq!(modifier_chord(modifier_bits::CTRL).vk, 17);
        assert_eq!(modifier_chord(modifier_bits::SHIFT).key, "Shift");
        assert_eq!(modifier_chord(modifier_bits::SHIFT).vk, 16);
        assert_eq!(modifier_chord(modifier_bits::ALT).key, "Alt");
        assert_eq!(modifier_chord(modifier_bits::META).key, "Meta");
    }

    // ── scroll_deltas（C2 视口滚动量换算，[纯逻辑]，CSS 像素零 DPR）─────────────────────

    #[test]
    fn scroll_deltas_direction_signs() {
        use crate::actions::ScrollDir;
        // Down → 视口下滚（deltaY > 0，内容上移）；Up → deltaY < 0。
        assert_eq!(scroll_deltas(ScrollDir::Down, Some(300.0)), (0.0, 300.0));
        assert_eq!(scroll_deltas(ScrollDir::Up, Some(300.0)), (0.0, -300.0));
        // Right → deltaX > 0；Left → deltaX < 0。
        assert_eq!(scroll_deltas(ScrollDir::Right, Some(150.0)), (150.0, 0.0));
        assert_eq!(scroll_deltas(ScrollDir::Left, Some(150.0)), (-150.0, 0.0));
    }

    #[test]
    fn scroll_deltas_default_step_when_amount_none() {
        use crate::actions::ScrollDir;
        // amount=None → 用 DEFAULT_SCROLL_STEP。
        assert_eq!(
            scroll_deltas(ScrollDir::Down, None),
            (0.0, DEFAULT_SCROLL_STEP)
        );
        assert_eq!(
            scroll_deltas(ScrollDir::Up, None),
            (0.0, -DEFAULT_SCROLL_STEP)
        );
    }

    #[test]
    fn scroll_deltas_negative_amount_is_taken_as_magnitude() {
        use crate::actions::ScrollDir;
        // 负 amount 取绝对值（方向由 dir 决定，量是标量）——防 LLM 给负值反转方向。
        assert_eq!(scroll_deltas(ScrollDir::Down, Some(-200.0)), (0.0, 200.0));
        assert_eq!(scroll_deltas(ScrollDir::Up, Some(-200.0)), (0.0, -200.0));
        assert_eq!(scroll_deltas(ScrollDir::Right, Some(-50.0)), (50.0, 0.0));
    }

    // ── chord_text（keyDown 该带的 text，[纯逻辑]，禁 DPR）─────────────────────────────

    #[test]
    fn chord_text_enter_is_carriage_return() {
        // Enter 必须带 "\r"，否则合成 keyDown 不触发隐式表单提交 / textarea 换行的默认动作。
        let c = parse_key_combo("Enter").unwrap();
        assert_eq!(chord_text(&c).as_deref(), Some("\r"));
    }

    #[test]
    fn chord_text_single_printable_char() {
        // 单个可打印字符 → 该字符（tier3 逐字符逃生的 keyDown 真产生 input）。
        assert_eq!(chord_text(&parse_key_combo("a").unwrap()).as_deref(), Some("a"));
        assert_eq!(chord_text(&parse_key_combo("5").unwrap()).as_deref(), Some("5"));
        assert_eq!(chord_text(&parse_key_combo("Space").unwrap()).as_deref(), Some(" "));
        // Shift+字母 → 大写（Shift 位还原大小写）。
        assert_eq!(chord_text(&parse_key_combo("Shift+a").unwrap()).as_deref(), Some("A"));
    }

    #[test]
    fn chord_text_named_keys_are_none() {
        // 导航/具名键无文本默认动作 → None。
        assert_eq!(chord_text(&parse_key_combo("Tab").unwrap()), None);
        assert_eq!(chord_text(&parse_key_combo("Escape").unwrap()), None);
        assert_eq!(chord_text(&parse_key_combo("ArrowDown").unwrap()), None);
    }

    #[test]
    fn chord_text_with_cmd_modifiers_is_none() {
        // Ctrl/Alt/Meta 修饰 = 快捷键，无文本默认动作（Ctrl+A 不该产生 'a' / Ctrl+Enter 不隐式提交）。
        assert_eq!(chord_text(&parse_key_combo("Ctrl+a").unwrap()), None);
        assert_eq!(chord_text(&parse_key_combo("Ctrl+Enter").unwrap()), None);
        assert_eq!(chord_text(&parse_key_combo("Alt+a").unwrap()), None);
        assert_eq!(chord_text(&parse_key_combo("Meta+Enter").unwrap()), None);
    }

    // ── ControlOrMeta 跨平台加速键 ─────────────────────────────────────────────
    #[test]
    fn control_or_meta_resolves_per_platform() {
        let chord = parse_key_combo("ControlOrMeta+A").unwrap();
        #[cfg(target_os = "macos")]
        assert_eq!(
            chord.modifiers,
            modifier_bits::META,
            "ControlOrMeta 在 mac 应解析为 Meta"
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            chord.modifiers,
            modifier_bits::CTRL,
            "ControlOrMeta 在非 mac 应解析为 Control"
        );
        // 别名同款。
        assert_eq!(parse_key_combo("CmdOrCtrl+A").unwrap().modifiers, chord.modifiers);
        assert_eq!(parse_key_combo("CtrlOrMeta+A").unwrap().modifiers, chord.modifiers);
    }

    // ── macEditingCommands（mac 专用：让键盘编辑命令经 CDP 真生效）──────────────
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_editing_commands_maps_and_strips_and_filters() {
        // Cmd+A → selectAll（去尾冒号）。
        assert_eq!(mac_editing_commands("KeyA", modifier_bits::META), vec!["selectAll"]);
        assert_eq!(mac_editing_commands("KeyC", modifier_bits::META), vec!["copy"]);
        assert_eq!(mac_editing_commands("KeyV", modifier_bits::META), vec!["paste"]);
        assert_eq!(mac_editing_commands("KeyZ", modifier_bits::META), vec!["undo"]);
        assert_eq!(
            mac_editing_commands("KeyZ", modifier_bits::SHIFT | modifier_bits::META),
            vec!["redo"]
        );
        // Ctrl+A 在 mac 是「移到段首」（不是全选！）——故 agent 全选必须用 ControlOrMeta(→Meta)。
        assert_eq!(
            mac_editing_commands("KeyA", modifier_bits::CTRL),
            vec!["moveToBeginningOfParagraph"]
        );
        // insert* 命令被过滤（由 insert_text 文本路径处理）：Enter→insertNewline 被滤掉 → 空。
        assert!(mac_editing_commands("Enter", 0).is_empty());
        // 多命令项（Control+KeyO 的 insertNewline* 被滤,只剩 moveBackward）。
        assert_eq!(
            mac_editing_commands("KeyO", modifier_bits::CTRL),
            vec!["moveBackward"]
        );
        // 修饰键顺序固定 Shift,Control,Alt,Meta（命中 "Shift+Control+KeyA"）。
        assert_eq!(
            mac_editing_commands("KeyA", modifier_bits::CTRL | modifier_bits::SHIFT),
            vec!["moveToBeginningOfParagraphAndModifySelection"]
        );
        // 无映射 → 空。
        assert!(mac_editing_commands("KeyW", modifier_bits::META).is_empty());
        assert!(mac_editing_commands("KeyA", 0).is_empty());
    }
}
