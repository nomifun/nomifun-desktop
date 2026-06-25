//! 序列化层安全：observe 产出的 aria YAML 在喂 LLM 前的纯文本处理。
//!
//! 两件事，安全一等公民：
//! 1. **脱敏**（[`redact_yaml`]）：剥掉 secret，LLM 永不见。委托 [`nomi_redact`] 的
//!    正则脱敏（sk-/AKIA/Bearer/k=v/PEM），再加「高熵 token」兜底——页面里那些
//!    没匹配已知模式、但又长又随机的串（session token、CSRF、JWT 片段等）。
//! 2. **不可信包裹**（[`wrap_untrusted`]）：页面文本是不可信数据，不是给 agent 的
//!    指令。用 `<data origin="…">…</data>` 包起来 + origin provenance，并把正文里的
//!    `</data>` 转义掉，堵「页面塞 `</data>` 提前闭合后注入指令」的越狱面。
//!
//! 纯逻辑、零 I/O、零 CDP。
//!
//! ## D5 password value 置空（[`blank_secret_values`]）
//! vendored PW `incrementalAriaSnapshot` 会把 `<input>`/`<textarea>` 的 `element.value`
//! 当单行内联文本子节点序列化进 aria YAML（如 `textbox "Password" [ref=f0e4]: hunter2`）。
//! password / autocomplete=password 字段的明文 value 因此**直接进 YAML 喂给 LLM**——违反
//! DESIGN §16 的 D5（LLM 永不见 secret）。脱敏正则 [`redact_yaml`] 对短/低熵口令（`hunter2`）
//! 无能为力。
//!
//! 修法：observe 逐帧拿快照后，在**同一 utility world contextId**（注入侧 `_ariaRef` expando
//! 只在该 world 可见）`Runtime.evaluate` 收集 **`input[type=password]`（及 autocomplete 含
//! password）** 对应元素的 aria `ref`（基于 **DOM 的 `type=password` 信号**，非文本启发式），
//! 宿主侧据此把这些 ref 所在 YAML 行的 `: value` 尾部抹成 `[REDACTED]`。脱敏仍在引擎序列化层
//! （不 fork vendored bundle，见 DESIGN §24）。

/// 浏览器序列化文本脱敏：委托 [`nomi_redact::redact_secrets`] + 高熵兜底。
///
/// 先跑已知模式正则（命中即替换为 `[REDACTED_SECRET]`），再对剩余文本逐 token 扫
/// 高熵串作兜底——两条路互补，任一命中即脱敏。
pub fn redact_yaml(yaml: &str) -> String {
    // nomi_redact 返回 Cow（无命中零分配）；高熵 pass 需 &str，借出即可。
    let base = nomi_redact::redact_secrets(yaml);
    redact_high_entropy_tokens(base.as_ref())
}

/// **D5 password value 置空**：把 `password_refs` 命中的 aria 行的内联 value（`: …` 尾部）
/// 抹成 `[REDACTED]`。基于 observe 在 utility world 收集的 **DOM `type=password`** 元素 ref，
/// 故只动真口令字段、不误伤普通 textbox。
///
/// aria 行形如 `  - textbox "Password" [ref=f0e4]: hunter2`。本函数找出 `password_refs` 命中的
/// 行，以**命中的 `[ref=<r>]` token 收尾处**为锚，把其后首个 `": "` 起的内联 value 整段截掉换成
/// ` [REDACTED]`（保留行内 role/name/`[ref=…]`/其它属性标记，只抹 value）。匹配 ref 用精确的
/// `[ref=<r>]` token（含括号），避免 `f0e4` 误配到 `f0e40`；锚在 ref token 处而非全行 `rfind(']')`，
/// 避免 password value 里含 `]` 时锚点落进 value 内部漏抹（见 [`blank_inline_value`]）。
///
/// 注意：该置空在缝合后、`wrap_untrusted` 前调用；ref 表/entries 用脱敏前 stitched 解析，
/// 不受影响（只动 value 文本，不动 ref）。
pub fn blank_secret_values(yaml: &str, password_refs: &[String]) -> String {
    if password_refs.is_empty() {
        return yaml.to_string();
    }
    yaml.lines()
        .map(|line| match matched_ref(line, password_refs) {
            Some(r) => blank_inline_value(line, r),
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 该行命中的 `[ref=<r>]`（精确 token 匹配，含 `]` 闭合）；无命中返回 `None`。
/// 返回命中的 ref 本体，供 [`blank_inline_value`] 用作锚点（避免再扫一遍）。
fn matched_ref<'a>(line: &str, refs: &'a [String]) -> Option<&'a str> {
    refs.iter()
        .map(String::as_str)
        .find(|r| line.contains(&format!("[ref={r}]")))
}

/// 抹掉 aria 行的内联 value：以 **`[ref=<r>]` token 的结束位置**为锚，从其后找首个 `": "`
/// 作 value 起点，把该 value 尾部整段换成 ` [REDACTED]`。无内联 value（行尾即属性收尾）则原样返回。
///
/// 形态：`  - textbox "Password" [ref=f0e4]: hunter2` → `  - textbox "Password" [ref=f0e4] [REDACTED]`。
///
/// **为何以 ref token 收尾处为锚、而非全行 `rfind(']')`**：password 是任意用户输入，value 里
/// 完全可能含 `]`（如 `pa]ss`、`my]secret`）。全行 `rfind(']')` 会把锚点落进 value 内部，其后
/// 再无 `": "` → 函数原样返回 → **明文泄露**（确定性 bug）。已知本行命中的就是 `[ref=<r>]`，
/// 故从该 token 之后找首个 `": "` 就是真正的内联值起点，不受 value 里的 `]` 干扰。
///
/// **保留 ref 后、`: ` 前的非-value 属性标记**（如 `[cursor=pointer]`）：这些不是 value，
/// `: ` 之前的部分原样保留，只抹 `: ` 之后的 value。value 里即便含 `:` 也整段抹掉。
fn blank_inline_value(line: &str, r: &str) -> String {
    // 锚 = `[ref=<r>]` token 在本行的结束位置（之后才是可能的属性标记 + 内联 value）。
    let token = format!("[ref={r}]");
    let Some(token_pos) = line.find(&token) else {
        // 理论不达（调用方已确认命中）；保守原样返回。
        return line.to_string();
    };
    let anchor = token_pos + token.len();
    let rest = &line[anchor..];
    let Some(colon_rel) = rest.find(": ") else {
        // ref token 之后无内联 value（行尾即属性收尾或 `]:` 后接子节点）→ 不动。
        return line.to_string();
    };
    let cut = anchor + colon_rel;
    format!("{} [REDACTED]", &line[..cut])
}

/// 兜底 over-redact（fail-closed）：把**所有可编辑控件行**的内联 `: value` 尾部一律抹成
/// ` [REDACTED]`，role ∈ {`textbox`,`searchbox`,`spinbutton`,`combobox`}。
///
/// 仅在 password 探测（[`crate::injected::InjectionManager::password_refs`]）**任一帧失败**时
/// 启用：此时无法精确知道哪些字段是 password，故对所有可编辑控件值整体置空，**绝不放行明文**
/// （Critical secret 控制须 fail-closed）。非编辑控件（button/link/heading/…）不动。
pub fn blank_all_editable_values(yaml: &str) -> String {
    yaml.lines()
        .map(|line| match editable_role(line) {
            // role token 收尾处之后找首个 `": "`，抹其 value（复用按 role 的内联值抹除）。
            Some(role) => blank_inline_value_after_role(line, role),
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 本行是否可编辑控件（role ∈ {textbox,searchbox,spinbutton,combobox}）；是则返回该 role。
/// 形如 `  - textbox "Password" [ref=f0e4]: …`：`- ` 后首 token 即 role。
fn editable_role(line: &str) -> Option<&'static str> {
    const EDITABLE: [&str; 4] = ["textbox", "searchbox", "spinbutton", "combobox"];
    let t = line.trim_start();
    let t = t.strip_prefix("- ")?;
    let role_end = t.find([' ', '"', '[']).unwrap_or(t.len());
    let role = &t[..role_end];
    EDITABLE.iter().copied().find(|&e| e == role)
}

/// 按 role token 定位锚再抹内联 value（[`blank_all_editable_values`] 用）。语义同
/// [`blank_inline_value`] 但锚是 role 出现处之后——可编辑控件未必有 `[ref=…]`（best-effort
/// 兜底口径，宁宽勿漏），故从 role 之后找首个 `": "`。
fn blank_inline_value_after_role(line: &str, role: &str) -> String {
    let Some(role_pos) = line.find(role) else {
        return line.to_string();
    };
    let anchor = role_pos + role.len();
    let rest = &line[anchor..];
    let Some(colon_rel) = rest.find(": ") else {
        return line.to_string();
    };
    let cut = anchor + colon_rel;
    format!("{} [REDACTED]", &line[..cut])
}

/// 把页面文本当不可信数据包裹：`<data origin="…">…</data>`。
///
/// 正文里的 `</data>` 转义成 `<\/data>`，确保最终文本里只有最外层那一个真闭合标签，
/// 防页面文本提前闭合 `<data>` 块后被当作给 agent 的指令（提示注入越狱）。
/// origin（来源页 URL）作为 provenance 写进属性，attr 里的 `"` 转义防属性逃逸。
pub fn wrap_untrusted(text: &str, origin: Option<&str>) -> String {
    let body = escape_data_close(text);
    match origin {
        Some(o) => format!("<data origin=\"{}\">\n{}\n</data>", escape_attr(o), body),
        None => format!("<data>\n{}\n</data>", body),
    }
}

/// 转义正文里的闭合标签：`</data>` → `<\/data>`，杜绝提前闭合越狱。
fn escape_data_close(s: &str) -> String {
    s.replace("</data>", "<\\/data>")
}

/// 转义属性值里的双引号，防 origin 注入额外属性 / 逃出引号。
fn escape_attr(s: &str) -> String {
    s.replace('"', "&quot;")
}

/// Shannon 熵（bits/char）。空串记 0.0。值越高字符分布越随机（secret 的特征）。
pub fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts = std::collections::HashMap::new();
    for c in s.chars() {
        *counts.entry(c).or_insert(0u32) += 1;
    }
    let len = s.chars().count() as f64;
    counts
        .values()
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

/// 高熵 token（长度 ≥ 20 且熵 ≥ 4.0）替换为 `[REDACTED_SECRET]`。
///
/// 用 `split_inclusive` 保留分隔符（空白 / `"`），重组后原排版不变；只替换 token 的
/// 「实体」部分（trim 掉两侧分隔），不动包裹引号 / 空白。
fn redact_high_entropy_tokens(s: &str) -> String {
    s.split_inclusive(|c: char| c.is_whitespace() || c == '"')
        .map(|tok| {
            let trimmed = tok.trim_matches(|c: char| c.is_whitespace() || c == '"');
            if trimmed.len() >= 20 && shannon_entropy(trimmed) >= 4.0 {
                tok.replace(trimmed, "[REDACTED_SECRET]")
            } else {
                tok.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_high_entropy_and_known_patterns() {
        let out = redact_yaml("- textbox \"API\": \"sk-abcdefghijklmnopqrstuvwxyz0123456789ABCD\"");
        assert!(!out.contains("sk-abcdefghijklmnopqrstuvwxyz"));
        let clean = redact_yaml("- button \"Submit order\" [ref=f0e1]");
        assert!(clean.contains("Submit order"));
    }

    #[test]
    fn wrap_untrusted_delimits_and_escapes() {
        let w = wrap_untrusted("- button \"X\"", Some("https://evil.com"));
        assert!(w.starts_with("<data"));
        assert!(w.trim_end().ends_with("</data>"));
        assert!(w.contains("origin=\"https://evil.com\""));
        let jail = wrap_untrusted("text </data> injected", None);
        assert_eq!(jail.matches("</data>").count(), 1); // 仅外层闭合
        assert!(jail.contains("<\\/data>"));
    }

    #[test]
    fn entropy_orders_correctly() {
        assert!(shannon_entropy("aaaa") < shannon_entropy("aB3$xK9zQp"));
    }

    #[test]
    fn blank_secret_values_redacts_only_matched_refs() {
        let yaml = "- textbox \"Password\" [ref=f0e4]: hunter2\n\
                    - textbox \"Email\" [ref=f0e3]: alice@example.com";
        // 只抹 f0e4（password ref），保留 f0e3（普通 textbox）。
        let out = blank_secret_values(yaml, &["f0e4".to_string()]);
        assert!(!out.contains("hunter2"), "password value leaked:\n{out}");
        assert!(out.contains("[ref=f0e4] [REDACTED]"), "expected redaction:\n{out}");
        assert!(out.contains("alice@example.com"), "non-password value must remain:\n{out}");
        // role/name/ref 不被破坏。
        assert!(out.contains("- textbox \"Password\" [ref=f0e4]"), "structure broken:\n{out}");
    }

    #[test]
    fn blank_secret_values_noop_when_no_refs() {
        let yaml = "- textbox \"Password\" [ref=f0e4]: hunter2";
        assert_eq!(blank_secret_values(yaml, &[]), yaml);
    }

    #[test]
    fn blank_secret_values_ref_token_is_exact() {
        // f0e4 不得误配 f0e40（精确 token，含 `]`）。
        let yaml = "- textbox \"A\" [ref=f0e40]: keepme";
        let out = blank_secret_values(yaml, &["f0e4".to_string()]);
        assert!(out.contains("keepme"), "f0e4 must not match f0e40:\n{out}");
    }

    #[test]
    fn blank_inline_value_leaves_valueless_lines() {
        // 无内联 value 的行（iframe / leaf）原样返回。
        let line = "  - iframe [ref=f0e5]";
        assert_eq!(blank_inline_value(line, "f0e5"), line);
        // 带 [cursor=pointer] 但无内联 value 的行也不动（行尾 `]:` 后接子节点，无 `: ` 内联值）。
        let line2 = "  - link \"X\" [ref=f2e9] [cursor=pointer]";
        assert_eq!(blank_inline_value(line2, "f2e9"), line2);
        let line3 = "    - link \"Inner\" [ref=f1e2] [cursor=pointer]:";
        assert_eq!(blank_inline_value(line3, "f1e2"), line3);
    }

    #[test]
    fn blank_inline_value_handles_colon_in_value() {
        // value 里含 `:`（如 url）也整段抹掉。
        let line = "- textbox \"URL\" [ref=f0e1]: https://x.test/a:b";
        let out = blank_inline_value(line, "f0e1");
        assert_eq!(out, "- textbox \"URL\" [ref=f0e1] [REDACTED]");
    }

    #[test]
    fn blank_inline_value_redacts_value_containing_bracket() {
        // 必修 A：password value 含 `]`（任意用户输入，`]` 合法）仍被抹——锚在 ref token 收尾处，
        // 不会被 value 里的 `]` 误导（旧全行 rfind(']') 会落进 value 内部漏抹 → 明文泄露）。
        assert_eq!(
            blank_inline_value("- textbox \"P\" [ref=f0e4]: pa]ss", "f0e4"),
            "- textbox \"P\" [ref=f0e4] [REDACTED]"
        );
        assert_eq!(
            blank_inline_value("- textbox \"P\" [ref=f0e4]: my]secret]value", "f0e4"),
            "- textbox \"P\" [ref=f0e4] [REDACTED]"
        );
    }

    #[test]
    fn blank_inline_value_preserves_nonvalue_brackets_then_redacts() {
        // ref 后存在非-value 属性标记（如 [cursor=pointer]）+ 含 `]` 的 value：标记保留、value 抹。
        assert_eq!(
            blank_inline_value("- textbox \"P\" [ref=f0e4] [cursor=pointer]: sec]ret", "f0e4"),
            "- textbox \"P\" [ref=f0e4] [cursor=pointer] [REDACTED]"
        );
    }

    #[test]
    fn blank_secret_values_redacts_value_with_bracket() {
        // 端到端经 blank_secret_values：含 `]` 的 password value 必不泄露。
        let yaml = "- textbox \"Password\" [ref=f0e4]: hun]ter2sk";
        let out = blank_secret_values(yaml, &["f0e4".to_string()]);
        assert!(!out.contains("hun]ter2"), "password with ] leaked:\n{out}");
        assert!(out.contains("[ref=f0e4] [REDACTED]"), "expected redaction:\n{out}");
    }

    #[test]
    fn blank_all_editable_values_redacts_editable_keeps_actionable() {
        let yaml = "- heading \"H\" [level=1] [ref=f0e1]\n\
                    - textbox \"User\" [ref=f0e2]: alice\n\
                    - searchbox \"Q\" [ref=f0e3]: my]query\n\
                    - spinbutton \"N\" [ref=f0e4]: 42\n\
                    - combobox \"C\" [ref=f0e5]: chosen\n\
                    - button \"Save\" [ref=f0e6]\n\
                    - link \"Home\" [ref=f0e7] [cursor=pointer]:";
        let out = blank_all_editable_values(yaml);
        // 可编辑控件值全抹（含 `]` 的也抹）。
        assert!(!out.contains("alice"), "textbox value leaked:\n{out}");
        assert!(!out.contains("my]query"), "searchbox value (with ]) leaked:\n{out}");
        assert!(!out.contains(": 42"), "spinbutton value leaked:\n{out}");
        assert!(!out.contains("chosen"), "combobox value leaked:\n{out}");
        assert!(out.contains("[ref=f0e2] [REDACTED]"), "textbox not redacted:\n{out}");
        // 非编辑控件原样保留（button/link/heading 不动）。
        assert!(out.contains("- heading \"H\" [level=1] [ref=f0e1]"), "heading touched:\n{out}");
        assert!(out.contains("- button \"Save\" [ref=f0e6]"), "button touched:\n{out}");
        assert!(out.contains("- link \"Home\" [ref=f0e7] [cursor=pointer]:"), "link touched:\n{out}");
    }
}
