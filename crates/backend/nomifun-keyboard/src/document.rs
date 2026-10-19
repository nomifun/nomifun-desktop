use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelSelection {
    pub provider_id: String,
    pub model: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub k2_enabled: bool,
    pub k3_enabled: bool,
    pub k4_enabled: bool,
    pub asr_model: Option<ModelSelection>,
    #[serde(default)]
    pub asr_mode: AsrMode,
    pub text_model: Option<ModelSelection>,
    pub language: String,
    #[serde(default)]
    pub audio_device_id: Option<String>,
    pub dictionary: Vec<String>,
    pub beautify_instruction: String,
    pub always_on_top: bool,
    #[serde(default = "default_subtitle_font_size")]
    pub subtitle_font_size: u16,
    pub launch_at_login: bool,
}
fn default_subtitle_font_size() -> u16 {
    14
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AsrMode {
    #[default]
    Streaming,
    Platform,
}
impl Default for Settings {
    fn default() -> Self {
        Self { enabled:true,k2_enabled:true,k3_enabled:true,k4_enabled:true,
        asr_model:None,asr_mode:AsrMode::Streaming,text_model:None,language:"zh".into(),audio_device_id:None,dictionary:vec![],
        beautify_instruction:"整理标点、语法、口头重复和段落，保留原意、事实、人名、数字、日期、代码和专有词。只输出整理后的完整正文。".into(),
        always_on_top:true,subtitle_font_size:default_subtitle_font_size(),launch_at_login:true }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !(12..=32).contains(&self.subtitle_font_size) {
            return Err("字幕字号必须是 12–32 的整数".into());
        }
        if self
            .audio_device_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 1024)
        {
            return Err("系统麦克风选择无效".into());
        }
        if self.dictionary.len() > 256
            || self
                .dictionary
                .iter()
                .any(|s| s.trim().is_empty() || s.len() > 256 || s.contains(['<', '>', '\r', '\n']))
        {
            return Err("词典最多 256 项，每项为不含尖括号的非空单行，且不超过 256 字节".into());
        }
        if self.language.len() > 64 || self.beautify_instruction.len() > 16384 {
            return Err("语言或美化指令过长".into());
        }
        for m in [&self.asr_model, &self.text_model].into_iter().flatten() {
            if m.provider_id.is_empty()
                || m.model.is_empty()
                || m.provider_id.len() > 256
                || m.model.len() > 256
            {
                return Err("模型引用无效".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Recording,
    Finalizing,
    Beautifying,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    Beautify,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub document_id: String,
    pub revision: u64,
    pub operation_id: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TextSelection {
    pub anchor: u64,
    pub focus: u64,
}
impl TextSelection {
    pub fn validate(&self, text: &str) -> Result<(), String> {
        self.byte_range(text).map(|_| ())
    }
    fn byte_range(&self, text: &str) -> Result<(usize, usize), String> {
        fn byte_index(text: &str, offset: u64) -> Option<usize> {
            let mut units = 0;
            for (index, character) in text.char_indices() {
                if units == offset {
                    return Some(index);
                }
                units += character.len_utf16() as u64;
                if units > offset {
                    return None;
                }
            }
            (units == offset).then_some(text.len())
        }
        let anchor = byte_index(text, self.anchor).ok_or("语音插入位置越界或拆分了 UTF-16 字符")?;
        let focus = byte_index(text, self.focus).ok_or("语音插入位置越界或拆分了 UTF-16 字符")?;
        Ok((anchor.min(focus), anchor.max(focus)))
    }
}
#[derive(Clone, Debug)]
struct DictationRange {
    start_byte: usize,
    end_byte: usize,
    start_utf16: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub document_id: Option<String>,
    pub revision: u64,
    pub operation_id: Option<String>,
    pub phase: Phase,
    pub text: String,
    pub original_text: String,
    pub partial: String,
    pub candidate: String,
    pub visible: bool,
    pub pending_action: Option<Intent>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub dictation_selection: Option<TextSelection>,
    #[serde(skip)]
    pub closed: bool,
    #[serde(skip)]
    audio_done: bool,
    #[serde(skip)]
    final_text: Option<String>,
    #[serde(skip)]
    close_after_stop: bool,
    #[serde(skip)]
    dictation_range: Option<DictationRange>,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            document_id: None,
            revision: 0,
            operation_id: None,
            phase: Phase::Idle,
            text: String::new(),
            original_text: String::new(),
            partial: String::new(),
            candidate: String::new(),
            visible: false,
            pending_action: None,
            error: None,
            notice: None,
            dictation_selection: None,
            closed: false,
            audio_done: false,
            final_text: None,
            close_after_stop: false,
            dictation_range: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    StopCapture,
    CancelCapture,
    CancelModel,
    Beautify(Identity, String),
    Show,
    Hide,
}
impl Document {
    /// Commit a manual edit only after recording and model work have settled.
    /// An unchanged renderer submission must not replace the restore baseline.
    pub fn edit(&mut self, text: String) -> Result<(), String> {
        if self.phase != Phase::Idle || self.closed {
            return Err("请先停止或取消当前操作，并打开当前字幕文稿".into());
        }
        if text.len() > 1024 * 1024 {
            return Err("字幕正文不能超过 1 MiB".into());
        }
        self.dictation_selection = None;
        self.dictation_range = None;
        if text == self.text {
            return Ok(());
        }
        let revision = self.revision.checked_add(1).ok_or("文稿版本已耗尽")?;
        if self.document_id.is_none() {
            self.document_id = Some(uuid::Uuid::now_v7().to_string());
        }
        self.text = text.clone();
        self.original_text = text;
        self.revision = revision;
        self.operation_id = None;
        self.partial.clear();
        self.candidate.clear();
        self.pending_action = None;
        self.error = None;
        self.notice = None;
        self.audio_done = false;
        self.final_text = None;
        self.close_after_stop = false;
        Ok(())
    }
    /// The caller must first prove native capture stopped and hiding succeeded.
    /// Close requests and failed stop/hide attempts keep their readable content.
    pub fn discard_closed(&mut self) {
        *self = Self {
            closed: true,
            ..Self::default()
        };
    }
    pub fn identity(&self) -> Option<Identity> {
        Some(Identity {
            document_id: self.document_id.clone()?,
            revision: self.revision,
            operation_id: self.operation_id.clone()?,
        })
    }
    pub fn matches(&self, id: &Identity) -> bool {
        self.identity().as_ref() == Some(id)
    }
    fn operation(&mut self) -> Identity {
        self.operation_id = Some(uuid::Uuid::now_v7().to_string());
        self.identity().expect("document exists")
    }
    pub fn begin_recording(&mut self) -> Result<Identity, String> {
        self.begin_recording_at(None)
    }
    pub fn begin_recording_at(
        &mut self,
        selection: Option<TextSelection>,
    ) -> Result<Identity, String> {
        if self.phase != Phase::Idle {
            return Err("请等待当前操作完成或取消".into());
        }
        let base = if self.document_id.is_none() || self.closed {
            ""
        } else {
            self.text.as_str()
        };
        let range = selection
            .map(|selection| {
                selection.byte_range(base).map(|(start_byte, end_byte)| DictationRange {
                    start_byte,
                    end_byte,
                    start_utf16: selection.anchor.min(selection.focus),
                })
            })
            .transpose()?;
        if self.document_id.is_none() || self.closed {
            *self = Self::default();
            self.document_id = Some(uuid::Uuid::now_v7().to_string());
        }
        self.closed = false;
        self.visible = true;
        self.error = None;
        self.notice = None;
        self.phase = Phase::Recording;
        self.audio_done = false;
        self.final_text = None;
        self.close_after_stop = false;
        self.pending_action = None;
        self.dictation_selection = selection;
        self.dictation_range = range;
        Ok(self.operation())
    }
    pub fn request(&mut self, intent: Intent) -> Vec<Effect> {
        self.error = None;
        self.notice = None;
        match self.phase {
            Phase::Recording => {
                self.phase = Phase::Finalizing;
                self.pending_action = Some(intent);
                vec![Effect::StopCapture]
            }
            Phase::Finalizing => {
                self.pending_action = Some(intent);
                vec![]
            }
            Phase::Beautifying => vec![],
            Phase::Idle => self.execute(intent),
        }
    }
    pub fn request_with_capture(
        &mut self,
        capture: Option<&Identity>,
        intent: Intent,
    ) -> Vec<Effect> {
        if capture.is_some_and(|id| !self.matches(id)) {
            self.notice = Some("等待系统麦克风停止确认".into());
            return vec![];
        }
        self.request(intent)
    }
    fn execute(&mut self, intent: Intent) -> Vec<Effect> {
        self.pending_action = None;
        if self.text.trim().is_empty() {
            self.notice = Some("暂无文字".into());
            return vec![];
        }
        let id = self.operation();
        match intent {
            Intent::Beautify => {
                self.dictation_selection = None;
                self.dictation_range = None;
                self.phase = Phase::Beautifying;
                self.candidate.clear();
                vec![Effect::Beautify(id, self.text.clone())]
            }
        }
    }
    pub fn stop(&mut self) -> Vec<Effect> {
        if self.phase == Phase::Recording {
            self.phase = Phase::Finalizing;
            vec![Effect::StopCapture]
        } else {
            vec![]
        }
    }
    pub fn partial(&mut self, id: &Identity, text: String) {
        if self.matches(id) && matches!(self.phase, Phase::Recording | Phase::Finalizing) {
            self.partial = text;
        }
    }
    pub fn audio_ended(&mut self, id: &Identity) -> Vec<Effect> {
        if !self.matches(id) || !matches!(self.phase, Phase::Recording | Phase::Finalizing) {
            return vec![];
        }
        self.audio_done = true;
        self.phase = Phase::Finalizing;
        self.settle_recording()
    }
    pub fn asr_finished(&mut self, id: &Identity, text: String) -> Vec<Effect> {
        if !self.matches(id) || !matches!(self.phase, Phase::Recording | Phase::Finalizing) {
            return vec![];
        }
        self.final_text = Some(text);
        self.settle_recording()
    }
    fn settle_recording(&mut self) -> Vec<Effect> {
        if !self.audio_done || self.final_text.is_none() {
            return vec![];
        }
        let text = self.final_text.take().unwrap();
        if let Some(range) = self.dictation_range.take() {
            self.dictation_selection = None;
            if !text.trim().is_empty() {
                if self.text.get(range.start_byte..range.end_byte).is_none() {
                    if let Some(id) = self.identity() {
                        self.fail(&id, "语音插入位置已失效".into());
                    }
                    return vec![];
                }
                self.text.replace_range(range.start_byte..range.end_byte, &text);
                self.original_text = self.text.clone();
                self.revision += 1;
                let end = range.start_utf16 + text.encode_utf16().count() as u64;
                self.dictation_selection = Some(TextSelection { anchor: end, focus: end });
            }
        } else if !text.trim().is_empty() {
            self.text.push_str(&text);
            self.original_text.push_str(&text);
            self.revision += 1;
        }
        self.partial.clear();
        self.phase = Phase::Idle;
        self.operation_id = None;
        if let Some(intent) = self.pending_action.take() {
            self.execute(intent)
        } else {
            vec![]
        }
    }
    pub fn candidate(&mut self, id: &Identity, text: String) {
        if self.matches(id) && self.phase == Phase::Beautifying {
            self.candidate = text;
        }
    }
    pub fn beautified(&mut self, id: &Identity, text: String) -> Vec<Effect> {
        if !self.matches(id) || self.phase != Phase::Beautifying {
            return vec![];
        }
        if text.trim().is_empty() {
            self.fail(id, "美化没有返回完整正文".into());
            return vec![];
        }
        self.text = text;
        self.dictation_selection = None;
        self.dictation_range = None;
        self.revision += 1;
        self.candidate.clear();
        self.phase = Phase::Idle;
        self.operation_id = None;
        self.pending_action = None;
        vec![]
    }
    pub fn fail(&mut self, id: &Identity, error: String) {
        if !self.matches(id) {
            return;
        }
        self.phase = Phase::Idle;
        self.operation_id = None;
        self.partial.clear();
        self.candidate.clear();
        self.pending_action = None;
        self.final_text = None;
        self.dictation_selection = None;
        self.dictation_range = None;
        self.error = Some(error);
    }
    pub fn cancel(&mut self, close: bool) -> Vec<Effect> {
        self.pending_action = None;
        self.partial.clear();
        self.candidate.clear();
        self.final_text = None;
        self.dictation_selection = None;
        self.dictation_range = None;
        self.operation_id = None;
        let recording = matches!(self.phase, Phase::Recording | Phase::Finalizing);
        self.phase = Phase::Idle;
        // Hide is gated by the runtime's positive capture-stopped receipt.
        self.close_after_stop = close;
        if !recording && close {
            self.visible = false;
        }
        let mut effects = vec![Effect::CancelModel];
        if recording {
            effects.push(Effect::CancelCapture);
        } else if close {
            effects.push(Effect::Hide);
        }
        effects
    }
    pub fn cancelled_capture(&mut self) -> Vec<Effect> {
        if self.close_after_stop {
            self.visible = false;
            self.close_after_stop = false;
            vec![Effect::Hide]
        } else {
            vec![]
        }
    }
    pub fn restore(&mut self) -> Result<(), String> {
        if self.phase != Phase::Idle {
            return Err("请先停止或取消当前操作".into());
        }
        self.text = self.original_text.clone();
        self.dictation_selection = None;
        self.dictation_range = None;
        self.revision += 1;
        self.error = None;
        Ok(())
    }
    pub fn open(&mut self) {
        if self.phase == Phase::Idle
            && ((self.closed && !self.visible) || self.document_id.is_none())
        {
            *self = Self::default();
            self.document_id = Some(uuid::Uuid::now_v7().to_string());
        }
        self.visible = true;
    }
    /// Resume an existing draft, or create a fresh editable document after close.
    pub fn open_for_edit(&mut self) -> Result<(), String> {
        if self.phase != Phase::Idle {
            return Err("请先停止或取消当前操作再编辑".into());
        }
        self.open();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_settings_default_to_streaming_and_platform_keeps_catalog_refs() {
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        value.as_object_mut().unwrap().remove("asrMode");
        value["asrModel"] =
            serde_json::json!({"providerId":"existing-provider","model":"existing-asr"});
        let saved: Settings = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(saved.asr_mode, AsrMode::Streaming);
        assert_eq!(saved.asr_model.unwrap().provider_id, "existing-provider");
        value["asrMode"] = serde_json::json!("platform");
        let platform: Settings = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(platform.asr_mode, AsrMode::Platform);
        platform.validate().unwrap();
        value["asrMode"] = serde_json::json!("unknown_automatic_fallback");
        assert!(serde_json::from_value::<Settings>(value).is_err());
    }
    fn recording() -> (Document, Identity) {
        let mut d = Document::default();
        let id = d.begin_recording().unwrap();
        (d, id)
    }
    #[test]
    fn recording_precedes_empty_beautify_and_waits_for_audio_and_final() {
        let (mut d, id) = recording();
        assert_eq!(d.request(Intent::Beautify), vec![Effect::StopCapture]);
        assert!(d.asr_finished(&id, "你好\n🙂".into()).is_empty());
        let e = d.audio_ended(&id);
        assert!(matches!(&e[..],[Effect::Beautify(_,s)] if s=="你好\n🙂"));
    }
    #[test]
    fn late_final_cannot_append_to_new_document() {
        let (mut d, old) = recording();
        d.cancel(true);
        d.cancelled_capture();
        let new = d.begin_recording().unwrap();
        d.request(Intent::Beautify);
        assert!(d.asr_finished(&old, "old".into()).is_empty());
        assert!(d.audio_ended(&old).is_empty());
        assert!(d.matches(&new));
        assert!(d.text.is_empty());
    }
    #[test]
    fn duplicate_final_and_eos_append_once() {
        let (mut d, id) = recording();
        d.audio_ended(&id);
        d.asr_finished(&id, "a".into());
        d.audio_ended(&id);
        d.asr_finished(&id, "a".into());
        assert_eq!(d.text, "a");
        assert_eq!(d.revision, 1);
    }
    #[test]
    fn incomplete_candidate_never_replaces_body_and_failure_preserves_original() {
        let (mut d, id) = recording();
        d.audio_ended(&id);
        d.asr_finished(&id, "原文".into());
        let e = d.request(Intent::Beautify);
        let Effect::Beautify(id, _) = &e[0] else {
            panic!()
        };
        let id = id.clone();
        d.candidate(&id, "半段".into());
        assert!(d.request(Intent::Beautify).is_empty());
        d.fail(&id, "truncated".into());
        assert_eq!(d.text, "原文");
        assert_eq!(d.pending_action, None);
        assert!(d.candidate.is_empty());
    }
    #[test]
    fn repeated_beautify_during_finalization_runs_once() {
        let (mut d, id) = recording();
        d.request(Intent::Beautify);
        d.request(Intent::Beautify);
        d.audio_ended(&id);
        assert!(matches!(
            &d.asr_finished(&id, "a".into())[..],
            [Effect::Beautify(_, _)]
        ));
    }
    #[test]
    fn close_waits_for_actual_stop() {
        let (mut d, _) = recording();
        let e = d.cancel(true);
        assert!(!e.contains(&Effect::Hide));
        assert!(d.visible);
        assert_eq!(d.cancelled_capture(), vec![Effect::Hide]);
    }
    #[test]
    fn cancel_drain_blocks_beautify_until_native_stop_is_confirmed() {
        let (mut d, id) = recording();
        d.audio_ended(&id);
        d.asr_finished(&id, "已定稿".into());
        let active = d.begin_recording().unwrap();
        d.cancel(false);
        assert!(
            d.request_with_capture(Some(&active), Intent::Beautify)
                .is_empty()
        );
        assert_eq!(d.text, "已定稿");
        assert!(!d.closed);
        assert!(
            matches!(&d.request_with_capture(None,Intent::Beautify)[..],[Effect::Beautify(_,text)] if text=="已定稿")
        );
    }
    #[test]
    fn manual_close_discards_only_after_stop_and_hide_then_starts_fresh() {
        let (mut d, id) = recording();
        d.audio_ended(&id);
        d.asr_finished(&id, "第一段".into());
        let document = d.document_id.clone();
        assert!(d.cancel(true).contains(&Effect::Hide));
        // A hide failure can still restore this text in the visible window.
        assert_eq!(d.text, "第一段");
        assert_eq!(d.document_id, document);
        d.discard_closed();
        let id = d.begin_recording().unwrap();
        d.audio_ended(&id);
        d.asr_finished(&id, "第二段".into());
        assert_ne!(d.document_id, document);
        assert_eq!(d.text, "第二段");
        assert_eq!(d.original_text, "第二段");
    }
    #[test]
    fn recording_close_preserves_finalized_text_until_positive_stop_and_hide() {
        let (mut d, id) = recording();
        d.audio_ended(&id);
        d.asr_finished(&id, "保留到停止成功".into());
        let active = d.begin_recording().unwrap();
        d.partial(&active, "尚未定稿".into());
        let effects = d.cancel(true);
        assert!(effects.contains(&Effect::CancelCapture));
        assert!(!effects.contains(&Effect::Hide));
        assert!(d.visible);
        assert!(!d.closed);
        assert_eq!(d.text, "保留到停止成功");
        assert_eq!(d.original_text, "保留到停止成功");
        assert!(d.asr_finished(&active, "迟到内容".into()).is_empty());
        // This callback represents actual native stop; hiding may still fail.
        assert_eq!(d.cancelled_capture(), vec![Effect::Hide]);
        assert_eq!(d.text, "保留到停止成功");
        d.discard_closed();
        assert!(d.text.is_empty() && d.original_text.is_empty());
        assert!(d.closed && !d.visible);
        assert!(d.asr_finished(&active, "迟到内容".into()).is_empty());
        assert!(d.audio_ended(&active).is_empty());
    }
    #[test]
    fn cancel_without_close_keeps_finalized_text_for_append() {
        let (mut d, id) = recording();
        d.audio_ended(&id);
        d.asr_finished(&id, "第一段".into());
        let document = d.document_id.clone();
        d.begin_recording().unwrap();
        d.cancel(false);
        assert!(d.cancelled_capture().is_empty());
        let next = d.begin_recording().unwrap();
        d.audio_ended(&next);
        d.asr_finished(&next, "第二段".into());
        assert_eq!(d.document_id, document);
        assert_eq!(d.text, "第一段第二段");
    }
    #[test]
    fn cancel_finalization_revokes_buffered_final_and_queued_beautify() {
        for final_before_cancel in [false, true] {
            let mut d = Document::default();
            d.edit("已保存的正文🙂".into()).unwrap();
            let document = d.document_id.clone();
            let revision = d.revision;
            let id = d.begin_recording().unwrap();
            d.partial(&id, "未完成字幕".into());
            d.request(Intent::Beautify);
            if final_before_cancel {
                assert!(d.asr_finished(&id, "本段不得提交".into()).is_empty());
            } else {
                assert!(d.audio_ended(&id).is_empty());
            }
            let effects = d.cancel(false);
            assert_eq!(effects, vec![Effect::CancelModel, Effect::CancelCapture]);
            assert!(d.cancelled_capture().is_empty());
            assert!(d.asr_finished(&id, "迟到最终结果".into()).is_empty());
            assert!(d.audio_ended(&id).is_empty());
            d.partial(&id, "迟到临时字幕".into());
            d.fail(&id, "迟到错误".into());
            assert_eq!(d.phase, Phase::Idle);
            assert_eq!(d.document_id, document);
            assert_eq!(d.revision, revision);
            assert_eq!(d.text, "已保存的正文🙂");
            assert_eq!(d.original_text, d.text);
            assert!(d.visible && !d.closed);
            assert!(d.pending_action.is_none() && d.operation_id.is_none());
            assert!(d.partial.is_empty() && d.candidate.is_empty() && d.error.is_none());
            assert!(d.final_text.is_none());
        }
    }
    #[test]
    fn cancel_beautify_keeps_full_body_and_rejects_every_late_model_event() {
        let mut d = Document::default();
        d.open();
        d.edit("用户刚编辑的完整正文".into()).unwrap();
        let revision = d.revision;
        let Effect::Beautify(old, _) = d.request(Intent::Beautify).remove(0) else {
            panic!()
        };
        d.candidate(&old, "未完成候选".into());
        assert_eq!(d.cancel(false), vec![Effect::CancelModel]);
        d.candidate(&old, "迟到候选".into());
        assert!(d.beautified(&old, "迟到结果".into()).is_empty());
        d.fail(&old, "迟到错误".into());
        assert_eq!(d.text, "用户刚编辑的完整正文");
        assert_eq!(d.original_text, d.text);
        assert_eq!(d.revision, revision);
        assert!(d.candidate.is_empty() && d.error.is_none());
        assert!(d.visible && !d.closed);
        let Effect::Beautify(current, _) = d.request(Intent::Beautify).remove(0) else {
            panic!()
        };
        assert_ne!(current.operation_id, old.operation_id);
        d.beautified(&old, "不得干扰重试".into());
        d.beautified(&current, "重试后的完整结果".into());
        assert_eq!(d.text, "重试后的完整结果");
    }
    #[test]
    fn edit_preserves_unicode_newlines_and_becomes_the_restore_baseline() {
        let mut d = Document::default();
        d.open();
        let document = d.document_id.clone();
        let edited = "  手动编辑🙂\r\n第二行\n\n代码 `x = 42`\n";
        d.edit(edited.into()).unwrap();
        assert_eq!(d.text, edited);
        assert_eq!(d.original_text, edited);
        assert_eq!(d.document_id, document);
        assert_eq!(d.revision, 1);
        let Effect::Beautify(polish, text) = d.request(Intent::Beautify).remove(0) else {
            panic!()
        };
        assert_eq!(text, edited);
        d.beautified(&polish, "美化全文".into());
        assert_eq!(d.original_text, edited);
        let revision = d.revision;
        d.edit(d.text.clone()).unwrap();
        assert_eq!(d.revision, revision);
        assert_eq!(d.original_text, edited);
        d.restore().unwrap();
        assert_eq!(d.text, edited);
    }
    #[test]
    fn edit_change_invalidates_old_results_and_clears_transient_actions() {
        let (mut d, id) = recording();
        d.audio_ended(&id);
        d.asr_finished(&id, "旧原文".into());
        let Effect::Beautify(polish, _) = d.request(Intent::Beautify).remove(0) else {
            panic!()
        };
        d.fail(&polish, "旧失败".into());
        d.partial = "旧临时字幕".into();
        d.candidate = "旧候选".into();
        d.pending_action = Some(Intent::Beautify);
        d.operation_id = Some(polish.operation_id.clone());
        d.notice = Some("旧提示".into());
        d.edit("新编辑正文".into()).unwrap();
        assert_eq!(d.revision, 2);
        assert!(d.operation_id.is_none() && d.pending_action.is_none());
        assert!(d.partial.is_empty() && d.candidate.is_empty());
        assert!(d.error.is_none() && d.notice.is_none());
        assert!(!d.audio_done && d.final_text.is_none() && !d.close_after_stop);
        assert!(d.beautified(&polish, "迟到美化".into()).is_empty());
        assert!(d.asr_finished(&id, "迟到 ASR".into()).is_empty());
        d.candidate(&polish, "迟到候选".into());
        d.fail(&polish, "迟到失败".into());
        assert_eq!(d.text, "新编辑正文");
        assert_eq!(d.original_text, "新编辑正文");
        assert!(d.candidate.is_empty() && d.error.is_none());
    }
    #[test]
    fn unchanged_edit_is_a_complete_noop() {
        let (mut d, id) = recording();
        d.audio_ended(&id);
        d.asr_finished(&id, "原文".into());
        let Effect::Beautify(polish, _) = d.request(Intent::Beautify).remove(0) else {
            panic!()
        };
        d.beautified(&polish, "美化文本".into());
        d.error = Some("保留错误".into());
        d.notice = Some("保留提示".into());
        let before = serde_json::to_value(&d).unwrap();
        d.edit(d.text.clone()).unwrap();
        assert_eq!(serde_json::to_value(&d).unwrap(), before);
        assert_eq!(d.original_text, "原文");
        assert!(d.audio_done);
    }
    #[test]
    fn edit_allows_empty_text_without_reusing_old_text() {
        let mut d = Document::default();
        d.edit("仅手打也可创建文稿".into()).unwrap();
        assert!(d.document_id.is_some());
        d.edit(String::new()).unwrap();
        assert_eq!(d.revision, 2);
        assert!(d.text.is_empty() && d.original_text.is_empty());
        assert!(d.request(Intent::Beautify).is_empty());
        assert_eq!(d.notice.as_deref(), Some("暂无文字"));
        let revision = d.revision;
        d.edit(String::new()).unwrap();
        assert_eq!(d.revision, revision);
    }
    #[test]
    fn edits_require_idle_open_document_and_enforce_utf8_byte_limit() {
        for phase in [Phase::Recording, Phase::Finalizing, Phase::Beautifying] {
            let mut d = Document { phase, ..Default::default() };
            let before = serde_json::to_value(&d).unwrap();
            assert!(d.edit("不可编辑".into()).is_err());
            assert_eq!(serde_json::to_value(&d).unwrap(), before);
        }
        let mut closed = Document { closed: true, ..Default::default() };
        assert!(closed.edit("已关闭".into()).is_err());
        let mut d = Document::default();
        d.edit("a".repeat(1024 * 1024)).unwrap();
        let before = serde_json::to_value(&d).unwrap();
        let mut too_long = d.text.clone();
        too_long.push('🙂');
        assert!(d.edit(too_long).is_err());
        assert_eq!(serde_json::to_value(&d).unwrap(), before);
    }
    #[test]
    fn opening_discarded_document_allocates_fresh_editable_identity() {
        let mut d = Document::default();
        d.open();
        let first = d.document_id.clone();
        assert!(first.is_some());
        d.edit("旧正文".into()).unwrap();
        d.discard_closed();
        assert!(d.closed);
        d.open();
        assert!(d.visible && !d.closed);
        assert!(d.document_id.is_some() && d.document_id != first);
        assert_eq!(d.revision, 0);
        assert!(d.text.is_empty() && d.original_text.is_empty());
        d.edit("新手打".into()).unwrap();
        let Effect::Beautify(_, text) = d.request(Intent::Beautify).remove(0) else {
            panic!()
        };
        assert_eq!(text, "新手打");
    }
    #[test]
    fn opening_during_active_recording_preserves_identity_and_content() {
        let (mut d, id) = recording();
        d.partial(&id, "当前录音".into());
        d.visible = false;
        d.open();
        assert!(d.visible);
        assert!(d.matches(&id));
        assert_eq!(d.phase, Phase::Recording);
        assert_eq!(d.partial, "当前录音");
    }
    #[test]
    fn open_for_edit_rejects_active_work_and_allocates_a_fresh_document_after_close() {
        let (mut active, id) = recording();
        assert!(active.open_for_edit().is_err());
        assert!(active.matches(&id));
        let mut d = Document::default();
        d.open();
        let old = d.document_id.clone();
        d.discard_closed();
        d.open_for_edit().unwrap();
        assert!(d.visible && !d.closed);
        assert_ne!(d.document_id, old);
        assert!(d.text.is_empty() && d.original_text.is_empty());
        let mut blank = Document::default();
        blank.open_for_edit().unwrap();
        assert!(blank.document_id.is_some() && blank.visible && !blank.closed);
    }
    #[test]
    fn text_selection_validates_utf16_boundaries_and_serializes_offsets() {
        let text = "中🙂\nAB";
        for offset in [0, 1, 3, 4, 5, 6] {
            TextSelection { anchor: offset, focus: offset }.validate(text).unwrap();
        }
        for selection in [
            TextSelection { anchor: 2, focus: 3 },
            TextSelection { anchor: 1, focus: 2 },
            TextSelection { anchor: 0, focus: 7 },
            TextSelection { anchor: u64::MAX, focus: 0 },
        ] {
            assert!(selection.validate(text).is_err());
        }
        let selection = TextSelection { anchor: 4, focus: 1 };
        let value = serde_json::to_value(selection).unwrap();
        assert_eq!(value, serde_json::json!({"anchor":4,"focus":1}));
        assert_eq!(serde_json::from_value::<TextSelection>(value).unwrap(), selection);
    }
    #[test]
    fn caret_dictation_waits_for_audio_and_final_then_inserts_unicode_once() {
        let mut d = Document::default();
        d.edit("甲🙂乙\n丁".into()).unwrap();
        let selection = TextSelection { anchor: 3, focus: 3 };
        let id = d.begin_recording_at(Some(selection)).unwrap();
        d.partial(&id, "临时字".into());
        assert_eq!(d.text, "甲🙂乙\n丁");
        assert_eq!(d.dictation_selection, Some(selection));
        assert!(d.asr_finished(&id, "新🙂\n行".into()).is_empty());
        assert_eq!(d.text, "甲🙂乙\n丁");
        assert!(d.audio_ended(&id).is_empty());
        assert_eq!(d.text, "甲🙂新🙂\n行乙\n丁");
        assert_eq!(d.original_text, d.text);
        assert_eq!(d.revision, 2);
        assert_eq!(d.dictation_selection, Some(TextSelection { anchor: 8, focus: 8 }));
        assert!(d.partial.is_empty());
        d.partial(&id, "迟到临时字".into());
        assert!(d.asr_finished(&id, "重复 final".into()).is_empty());
        assert!(d.audio_ended(&id).is_empty());
        assert_eq!(d.text, "甲🙂新🙂\n行乙\n丁");
        assert_eq!(d.revision, 2);
        assert!(d.partial.is_empty());
    }
    #[test]
    fn reverse_selection_dictation_replaces_only_the_captured_range_verbatim() {
        let mut d = Document::default();
        d.edit("甲🙂乙\n丁".into()).unwrap();
        let id = d.begin_recording_at(Some(TextSelection { anchor: 4, focus: 1 })).unwrap();
        d.audio_ended(&id);
        d.asr_finished(&id, " 新\r\n🙂 ".into());
        assert_eq!(d.text, "甲 新\r\n🙂 \n丁");
        assert_eq!(d.original_text, d.text);
        assert_eq!(d.dictation_selection, Some(TextSelection { anchor: 8, focus: 8 }));
    }
    #[test]
    fn empty_or_whitespace_final_preserves_selected_text_and_baseline() {
        for final_text in ["", " \r\n\t"] {
            let mut d = Document::default();
            d.edit("选中的🙂\n正文".into()).unwrap();
            let text = d.text.clone();
            let revision = d.revision;
            let id = d.begin_recording_at(Some(TextSelection { anchor: 0, focus: 5 })).unwrap();
            d.audio_ended(&id);
            d.asr_finished(&id, final_text.into());
            assert_eq!(d.text, text);
            assert_eq!(d.original_text, text);
            assert_eq!(d.revision, revision);
            assert_eq!(d.phase, Phase::Idle);
            assert!(d.dictation_selection.is_none() && d.dictation_range.is_none());
        }
    }
    #[test]
    fn invalid_dictation_selection_has_no_document_side_effects() {
        let mut d = Document::default();
        d.edit("前🙂后".into()).unwrap();
        let before = serde_json::to_value(&d).unwrap();
        for selection in [TextSelection { anchor: 2, focus: 2 }, TextSelection { anchor: 0, focus: 5 }] {
            assert!(d.begin_recording_at(Some(selection)).is_err());
            assert_eq!(serde_json::to_value(&d).unwrap(), before);
        }
        let id = d.begin_recording_at(Some(TextSelection { anchor: 3, focus: 3 })).unwrap();
        assert!(d.begin_recording_at(None).is_err());
        assert!(d.matches(&id));
    }
    #[test]
    fn cancelled_and_late_dictation_cannot_replace_text_in_a_new_operation() {
        let mut d = Document::default();
        d.edit("保留正文".into()).unwrap();
        let old = d.begin_recording_at(Some(TextSelection { anchor: 0, focus: 4 })).unwrap();
        d.cancel(false);
        assert!(d.dictation_selection.is_none() && d.dictation_range.is_none());
        assert_eq!(d.text, "保留正文");
        let current = d.begin_recording_at(Some(TextSelection { anchor: 2, focus: 2 })).unwrap();
        assert!(d.asr_finished(&old, "旧结果".into()).is_empty());
        assert!(d.audio_ended(&old).is_empty());
        d.fail(&old, "迟到失败".into());
        d.audio_ended(&current);
        d.asr_finished(&current, "新".into());
        assert_eq!(d.text, "保留新正文");
        assert_eq!(d.dictation_selection, Some(TextSelection { anchor: 3, focus: 3 }));
    }
    fn inserted_document() -> Document {
        let mut d = Document::default();
        d.edit("ab".into()).unwrap();
        let id = d.begin_recording_at(Some(TextSelection { anchor: 1, focus: 1 })).unwrap();
        d.audio_ended(&id);
        d.asr_finished(&id, "🙂".into());
        assert_eq!(d.dictation_selection, Some(TextSelection { anchor: 3, focus: 3 }));
        d
    }
    #[test]
    fn dictation_metadata_is_cleared_by_manual_or_model_operations() {
        let mut edited = inserted_document();
        let revision = edited.revision;
        edited.edit(edited.text.clone()).unwrap();
        assert_eq!(edited.revision, revision);
        assert!(edited.dictation_selection.is_none());
        let mut restored = inserted_document();
        restored.restore().unwrap();
        assert!(restored.dictation_selection.is_none());
        let mut polished = inserted_document();
        polished.request(Intent::Beautify);
        assert!(polished.dictation_selection.is_none());
        for close in [false, true] {
            let mut cancelled = inserted_document();
            cancelled.cancel(close);
            assert!(cancelled.dictation_selection.is_none());
        }
        let mut failed = inserted_document();
        let id = failed.begin_recording_at(Some(TextSelection { anchor: 0, focus: 1 })).unwrap();
        failed.fail(&id, "识别失败".into());
        assert!(failed.dictation_selection.is_none() && failed.dictation_range.is_none());
        assert_eq!(failed.text, "a🙂b");
    }
    #[test]
    fn closed_dictation_does_not_reuse_the_old_document_selection() {
        let mut d = inserted_document();
        let old_document = d.document_id.clone();
        d.discard_closed();
        assert!(d.dictation_selection.is_none() && d.dictation_range.is_none());
        assert!(d.begin_recording_at(Some(TextSelection { anchor: 3, focus: 3 })).is_err());
        assert!(d.closed && d.document_id.is_none() && d.text.is_empty());
        let next = d.begin_recording_at(Some(TextSelection { anchor: 0, focus: 0 })).unwrap();
        assert_ne!(d.document_id, old_document);
        d.audio_ended(&next);
        d.asr_finished(&next, "新稿".into());
        assert_eq!(d.text, "新稿");
        assert_eq!(d.dictation_selection, Some(TextSelection { anchor: 2, focus: 2 }));
    }
    #[test]
    fn fallback_continues_without_a_separator_and_keeps_the_original_baseline() {
        let (mut d, id) = recording();
        d.audio_ended(&id);
        d.asr_finished(&id, "第一段".into());
        let Effect::Beautify(polish, _) = d.request(Intent::Beautify).remove(0) else { panic!() };
        d.beautified(&polish, "第一段美化".into());
        let next = d.begin_recording().unwrap();
        d.audio_ended(&next);
        d.asr_finished(&next, "第二段".into());
        assert_eq!(d.text, "第一段美化第二段");
        assert_eq!(d.original_text, "第一段第二段");
        assert!(d.dictation_selection.is_none());
    }
    #[test]
    fn fallback_keeps_provider_and_user_line_breaks_without_inserting_its_own() {
        let mut d = Document::default();
        d.edit("用户首行\r\n用户第二行：".into()).unwrap();
        let first = d.begin_recording().unwrap();
        d.audio_ended(&first);
        d.asr_finished(&first, " 模型第一行\n模型第二行".into());
        let second = d.begin_recording().unwrap();
        d.audio_ended(&second);
        d.asr_finished(&second, "\r\n下一段🙂\n".into());
        assert_eq!(d.text, "用户首行\r\n用户第二行： 模型第一行\n模型第二行\r\n下一段🙂\n");
        assert_eq!(d.original_text, d.text);
        assert_eq!(d.revision, 3);
    }
    #[test]
    fn existing_settings_default_to_14px_without_replacing_other_values() {
        let mut settings = Settings::default();
        settings.enabled = false;
        settings.k3_enabled = false;
        settings.asr_mode = AsrMode::Platform;
        settings.audio_device_id = Some("existing-device".into());
        settings.dictionary = vec!["已有专有词".into()];
        settings.beautify_instruction = "已有指令".into();
        settings.asr_model = Some(ModelSelection { provider_id: "provider-ref".into(), model: "model-ref".into() });
        let mut value = serde_json::to_value(&settings).unwrap();
        assert_eq!(value["subtitleFontSize"], 14);
        value.as_object_mut().unwrap().remove("subtitleFontSize");
        let loaded: Settings = serde_json::from_value(value).unwrap();
        loaded.validate().unwrap();
        assert_eq!(loaded.subtitle_font_size, 14);
        assert_eq!(serde_json::to_value(loaded).unwrap(), serde_json::to_value(settings).unwrap());
    }
    #[test]
    fn subtitle_font_size_is_an_integer_in_the_supported_range() {
        for size in [12, 14, 24, 32] {
            let mut settings = Settings::default();
            settings.subtitle_font_size = size;
            settings.validate().unwrap();
            let loaded: Settings = serde_json::from_value(serde_json::to_value(settings).unwrap()).unwrap();
            assert_eq!(loaded.subtitle_font_size, size);
        }
        for size in [0, 11, 33, u16::MAX] {
            let mut settings = Settings::default();
            settings.subtitle_font_size = size;
            assert!(settings.validate().is_err());
        }
        for invalid in [serde_json::json!(-1), serde_json::json!(14.5), serde_json::json!("14"), serde_json::json!(65536)] {
            let mut value = serde_json::to_value(Settings::default()).unwrap();
            value["subtitleFontSize"] = invalid;
            assert!(serde_json::from_value::<Settings>(value).is_err());
        }
    }
}
