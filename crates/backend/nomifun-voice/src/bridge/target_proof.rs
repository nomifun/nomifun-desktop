//! ASR arrival is not a capture clock. This proof is local to one activation
//! and never selects a successor task or creates canonical authority.
use super::*;
use nomifun_voice_contracts::{VoiceInputSourceRef,VoiceContextPresentation,VoiceSourceContextRequirement};

#[derive(Clone)]
pub(super) enum SourceTargetProof {Frozen(Option<WorkTarget>),Ambiguous}
#[derive(Clone)]
pub(super) struct ConfirmedSourceTarget {pub revision:u64,pub target:WorkTarget}

impl VoiceWorkBridge {
    fn presentation_target(&self)->Option<WorkTarget> {
        self.observed_target.clone().or_else(||{
            let mut approvals=self.approvals.values();let first=approvals.next()?;
            approvals.next().is_none().then(||first.0.target.work_target.clone())
        })
    }
    pub fn context_presentation(&self)->VoiceContextPresentation {
        let target=self.presentation_target();
        let digest=nomifun_agent_contracts::digest_payload(&(self.voice_session_id.as_str(),self.epoch,self.agent_session_id.as_str(),self.binding_version,self.context_floor,self.context_revision,&target)).expect("bounded application work context is serializable");
        VoiceContextPresentation {context_key:format!("voice-context:{}",digest.as_ref()),target}
    }
    pub fn required_source_context(&self)->Option<VoiceSourceContextRequirement> {
        self.required_source_trigger.as_ref()?;
        self.required_source.as_ref().map(|source_ref|VoiceSourceContextRequirement {source_ref:source_ref.clone(),context:self.context_presentation()})
    }
    /// The service calls this only after authenticated, explicit user
    /// confirmation of the shown task. A vendor tool cannot mint this proof.
    pub fn confirm_source_context(&mut self,source_ref:VoiceInputSourceRef,context_key:&str)->Result<(),VoiceError> {
        let context=self.context_presentation();
        if context.context_key!=context_key||self.required_source.as_ref()!=Some(&source_ref) {
            return Err(invalid("source confirmation does not match the current application presentation"));
        }
        if !self.transcripts.get(&source_ref.fragment_id).is_some_and(|source|source.speaker==VoiceSpeaker::User&&source.commit==TranscriptCommit::Committed&&source.revision==source_ref.revision) {
            return Err(invalid("source confirmation requires its current committed user revision"));
        }
        let target=context.target.ok_or_else(||invalid("there is no exact task to confirm"))?;
        let trigger=self.required_source_trigger.as_ref().ok_or_else(||invalid("there is no unreserved source trigger to confirm"))?;
        let WorkTrigger::TypedToolCall {arguments,..}=trigger else{return Err(invalid("this source trigger needs its proven media timeline"));};
        let mut payload=arguments.clone();if let Some(fields)=payload.as_object_mut(){fields.remove("source_ref");}
        let request=self.typed_request(payload)?;
        let requested=match &request {VoiceWorkRequest::Steer{target,..}|VoiceWorkRequest::Cancel{target}|VoiceWorkRequest::Observe{target}=>target,VoiceWorkRequest::AnswerApproval{answer}=>&answer.target.work_target,_=>return Err(invalid("source confirmation cannot change its original action"))};
        if requested!=&target{return Err(invalid("source confirmation cannot retarget the original trigger"));}
        let ids=vec![source_ref.fragment_id.clone()];
        if let Some(affinity)=self.original_source_target(&ids)? {
            if affinity!=target{return Err(invalid("an admitted source cannot be redirected to another task"));}
        }
        self.confirmed_source_targets.insert(source_ref.fragment_id,ConfirmedSourceTarget {revision:source_ref.revision,target});
        self.required_source=None;self.confirmed_source_trigger=self.required_source_trigger.take();Ok(())
    }
    /// Re-prepare only: the failed preparation never reserved a work link.
    /// No automatic canonical replay occurs from this getter.
    pub fn take_confirmed_source_trigger(&mut self)->Option<WorkTrigger>{self.confirmed_source_trigger.take()}
    pub(super) fn target_context_changed(&mut self) {
        self.context_revision=self.context_revision.saturating_add(1);
        self.required_source=None;self.required_source_trigger=None;self.confirmed_source_trigger=None;
    }
    pub(super) fn freeze_source_target(&self,fragment:&TranscriptFragment)->SourceTargetProof {
        if let Some(range)=fragment.media_range.as_ref().filter(|range|range.end_us>range.start_us) {
            let windows=self.windows.keys().filter(|id|fragment.fragment_id.starts_with(&format!("{id}:"))).collect::<Vec<_>>();
            if windows.len()==1 {
                return match self.target_for_range(windows[0],range){Ok(target)=>SourceTargetProof::Frozen(target),Err(_)=>SourceTargetProof::Ambiguous};
            }
        }
        if self.context_revision==0 {SourceTargetProof::Frozen(self.presentation_target())}else{SourceTargetProof::Ambiguous}
    }
    fn original_source_target(&self,ids:&[String])->Result<Option<WorkTarget>,VoiceError> {
        let mut target=None;
        for source in self.trigger_sources.values().filter(|source|source.fragments.iter().any(|id|ids.contains(id))) {
            if let Some(original)=&source.bound_target {
                if target.as_ref().is_some_and(|prior|prior!=original){return Err(invalid("source names multiple original task observations; resolve its canonical receipt"));}
                target=Some(original.clone());
            }
        }Ok(target)
    }
    pub(super) fn guard_source_target(&mut self,request:&VoiceWorkRequest,ids:&[String],text:&str,_context_key:Option<&str>)->Result<(),VoiceError> {
        if matches!(request,VoiceWorkRequest::Start{..}) {
            if relative_control(text){return Err(invalid("a relative task control cannot become a new task"));}return Ok(());
        }
        let target=match request {
            VoiceWorkRequest::Cancel{target}|VoiceWorkRequest::Steer{target,..}|VoiceWorkRequest::Observe{target}=>target,
            VoiceWorkRequest::AnswerApproval{answer}=>&answer.target.work_target,
            _=>return Ok(()),
        };
        if ids.is_empty()&&matches!(request,VoiceWorkRequest::Observe{..}){return Ok(());}
        if let Some(original)=self.original_source_target(ids)? {
            return if original==*target {Ok(())}else{Err(invalid("accepted source is bound to its original canonical task"))};
        }
        if explicitly_named(request,text){return Ok(());}
        for id in ids {
            let source=self.transcripts.get(id).ok_or_else(||invalid("work source disappeared"))?;
            if let Some(confirmed)=self.confirmed_source_targets.get(id).filter(|proof|proof.revision==source.revision) {
                if confirmed.target!=*target{return Err(invalid("confirmed source target differs from its exact work request"));}
                continue;
            }
            if let Some(SourceTargetProof::Frozen(frozen))=self.source_target_proofs.get(id) {
                if frozen.as_ref()==Some(target){continue;}
                // This source already has a proved context. User confirmation
                // cannot silently redirect a relative instruction to B.
                if !matches!(request,VoiceWorkRequest::AnswerApproval{..})||self.context_revision!=0 {
                    return Err(invalid("the committed source is frozen to another task or to no task"));
                }
            }
            // A question-specific answer in an unchanged activation has the
            // original actual presentation, even if no Turn is running.
            if self.context_revision==0&&matches!(request,VoiceWorkRequest::AnswerApproval{..}){continue;}
            if self.context_presentation().target.as_ref()!=Some(target){return Err(invalid("unknown source clock needs an explicit task name; the supplied target is not the current presentation"));}
            self.required_source=Some(VoiceInputSourceRef {fragment_id:id.clone(),revision:source.revision});
            return Err(VoiceError::new(VoiceErrorKind::SourceContextRequired,"the source clock cannot prove this task; confirm the application-presented exact context"));
        }Ok(())
    }
}

fn relative_control(text:&str)->bool {
    let compact=intent_text(text);
    matches!(compact.as_str(),"取消当前任务"|"停止当前任务"|"取消这个任务"|"停止这个任务"|"cancelthecurrenttask"|"stopthecurrenttask"|"cancelthistask"|"stopthistask")
        ||correction_intent(text)
}
pub(super) fn correction_intent(text:&str)->bool {
    let compact=intent_text(text);
    compact.starts_with("改成")||compact.starts_with("纠正")||compact.starts_with("changeto")||compact.starts_with("correct")
        ||compact.strip_prefix("不是").is_some_and(|rest|rest.contains('是'))
        ||["帮我将","请将","请把","帮我把","把"].iter().any(|prefix|compact.starts_with(prefix))
            &&["改到","改成","改为","修改","调整"].iter().any(|verb|compact.contains(verb))
}
fn explicitly_named(request:&VoiceWorkRequest,text:&str)->bool {
    if let VoiceWorkRequest::AnswerApproval{answer}=request {return text.contains(&answer.target.presentation_id);}
    let(target,verbs)=match request {
        VoiceWorkRequest::Cancel{target}=>(target,&["取消任务","停止任务","cancel task ","stop task ","cancel the original task "][..]),
        VoiceWorkRequest::Steer{target,..}=>(target,&["纠正任务","修改任务","correct task ","change task "][..]),
        VoiceWorkRequest::Observe{target}=>(target,&["查询任务","查看任务","observe task ","query task "][..]),
        _=>return false,
    };
    let text=text.trim().trim_start_matches("请");
    let text=if text.get(..7).is_some_and(|prefix|prefix.eq_ignore_ascii_case("please ")){&text[7..]}else{text};
    // Fold only the action wording. Operation identifiers remain exact bytes.
    verbs.iter().any(|verb|text.get(..verb.len()).filter(|prefix|prefix.eq_ignore_ascii_case(verb)).and_then(|_|text.get(verb.len()..)).map(str::trim_start).is_some_and(|rest|rest.strip_prefix(target.turn_operation_id.as_ref()).is_some_and(|tail|{
        if matches!(request,VoiceWorkRequest::Steer{..}) {return tail.starts_with(|character:char|character.is_whitespace()||matches!(character,'：'|':'|'，'|','))&&!tail.trim_matches(|character:char|character.is_whitespace()||matches!(character,'：'|':'|'，'|',')).is_empty();}
        tail.trim().trim_end_matches(['。','.','！','!']).is_empty()
    })))
}
