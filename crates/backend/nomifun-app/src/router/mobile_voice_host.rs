//! Dedicated product media/control surface, separate from the business event WebSocket.
use std::sync::Arc;
use std::time::Duration;
use axum::{Router, Json, Extension};
use axum::extract::{State, Path, Query, WebSocketUpgrade, ws::{Message,WebSocket}};
use axum::http::HeaderMap;
use axum::routing::{get,post};
use axum::response::Response;
use futures_util::{StreamExt,SinkExt};
use nomifun_voice_contracts::voice::*;
use nomifun_api_types::ApiResponse;
use nomifun_auth::CurrentUser;
use nomifun_common::AppError;
use nomifun_voice::{VoiceSessionService,VoiceActivationRequest,VoiceActivationResponse,VoiceMediaAttachment,VoiceSessionProjection};
use serde::{Serialize,Deserialize};
use super::mobile_voice_registry::AppVoiceRegistry;
use super::mobile_voice_authority::AppVoiceAuthority;
use nomifun_voice_contracts::{VoiceProfile,VoiceProfileUpdate,VoiceWorkCommand};

async fn capabilities()->Json<ApiResponse<serde_json::Value>>{Json(ApiResponse::ok(serde_json::json!({"supported":true,"api_version":1,"media_version":"NFV1","media_subprotocol":"nomifun.voice.v1","requires_foreground_lease":true,"foreground_lease_ttl_ms":3000,"input_credit":true,"transports":["relay","native_webrtc"]})))}
async fn save_profile(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,Path(id):Path<String>,Json(update):Json<VoiceProfileUpdate>)->Result<Json<ApiResponse<VoiceProfile>>,AppError>{
    let current=state.authority.live(user.id.as_ref(),update.agent_session_id.as_ref()).await.map_err(error)?;
    if current.agent_binding.binding_version!=update.binding_version{return Err(AppError::Conflict("voice profile binding is stale".into()));}
    let revision=update.expected_revision.checked_add(1).ok_or_else(||AppError::BadRequest("voice profile revision overflow".into()))?;
    let route=if update.enabled{state.registry.resolve_record(&update,revision).await.map_err(error)?}else{
        let saved=state.authority.journal.get_or_open().await.map_err(error)?.profile(user.id.as_ref().into(),id.clone()).await.map_err(error)?.ok_or_else(||AppError::BadRequest("voice profile does not exist".into()))?;
        if saved.agent_session_id!=update.agent_session_id{return Err(AppError::Conflict("voice profile belongs to a different Session".into()));}
        saved.route
    };
    let profile=VoiceProfile{profile_id:id,revision,agent_session_id:update.agent_session_id,binding_version:update.binding_version,enabled:update.enabled,label:update.model,route,work_steering_policy:update.work_steering_policy};
    let profile=state.authority.journal.get_or_open().await.map_err(error)?.save_profile(user.id.as_ref().into(),profile,update.expected_revision).await.map_err(error)?;
    Ok(Json(ApiResponse::ok(profile)))
}
async fn work_command(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,Path(id):Path<String>,Json(command):Json<VoiceWorkCommand>)->Result<Json<ApiResponse<VoiceWorkReceipt>>,AppError>{
    let receipt=state.service.work_command(user.id.as_ref(),&id,command).await.map_err(error)?;Ok(Json(ApiResponse::ok(receipt)))
}
async fn confirm_source_context(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,Path(id):Path<String>,Json(confirmation):Json<nomifun_voice_contracts::VoiceSourceContextConfirmation>)->Result<Json<ApiResponse<serde_json::Value>>,AppError>{
    state.service.confirm_source_context(user.id.as_ref(),&id,confirmation).await.map_err(error)?;
    Ok(Json(ApiResponse::ok(serde_json::json!({"confirmed":true}))))
}
async fn probe_profile(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,local:Option<Extension<nomifun_auth::trust::LocalTrusted>>,headers:HeaderMap,Path(id):Path<String>,Json(request):Json<nomifun_voice_contracts::VoiceProfileProbeRequest>)->Result<Json<ApiResponse<nomifun_voice_contracts::VoiceProfileProbeResult>>,AppError>{
    let access=auth_lease(&state,&headers,user.id.as_ref(),local.is_some())?;
    Ok(Json(ApiResponse::ok(state.service.probe_profile(user.id.as_ref(),&id,request,access).await.map_err(error)?)))
}
async fn export_data(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>)->Result<Response,AppError>{
    let exported=state.service.export_owned_data(user.id.as_ref()).await.map_err(error)?;
    axum::http::Response::builder().header("content-type","application/vnd.sqlite3")
        .header("content-disposition","attachment; filename=voice.snapshot.sqlite3")
        .header("cache-control","no-store").header("x-nomifun-voice-schema",exported.boundary.schema_version.to_string())
        .header("x-nomifun-voice-sequence",exported.boundary.max_sequence.to_string())
        .body(axum::body::Body::from(exported.bytes)).map_err(|_|AppError::Internal("voice export response failed".into()))
}
async fn delete_data(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,Path(session):Path<String>)->Result<Json<ApiResponse<serde_json::Value>>,AppError>{
    state.service.delete_owned_data(user.id.as_ref(),&session,true).await.map_err(error)?;
    Ok(Json(ApiResponse::ok(serde_json::json!({"voice_data_deleted":true,"agent_session_id":session}))))
}
enum HttpAccessLease{
    Installation{validator:Arc<nomifun_auth::InstanceTokenValidator>,generation:u64},
    Jwt{service:Arc<nomifun_auth::JwtService>,token:zeroize::Zeroizing<String>,owner:String},
    Local,
}
impl nomifun_voice::VoiceAccessLease for HttpAccessLease{
    fn is_valid(&self)->bool{match self{
        Self::Installation{validator,generation}=>validator.is_configured()&&validator.generation()==*generation,
        Self::Jwt{service,token,owner}=>service.verify(token.as_str()).is_ok_and(|payload|payload.user_id.as_ref()==owner.as_str()),
        Self::Local=>true,
    }}
}
fn auth_lease(state:&VoiceRouterState,headers:&HeaderMap,owner:&str,local_trusted:bool)->Result<Arc<dyn nomifun_voice::VoiceAccessLease>,AppError>{
    if let Some(token)=nomifun_auth::extract_token_from_headers(headers){
        let generation=state.token_validator.generation();
        if state.token_validator.validate(&token){if state.token_validator.generation()!=generation{return Err(AppError::Forbidden("voice authentication rotated during admission".into()));}return Ok(Arc::new(HttpAccessLease::Installation{validator:state.token_validator.clone(),generation}));}
        let payload=state.jwt.verify(&token).map_err(|_|AppError::Forbidden("voice authentication lease is invalid".into()))?;
        if payload.user_id.as_ref()!=owner{return Err(AppError::Forbidden("voice authentication owner differs".into()));}
        return Ok(Arc::new(HttpAccessLease::Jwt{service:state.jwt.clone(),token:zeroize::Zeroizing::new(token),owner:owner.into()}));
    }
    if local_trusted{return Ok(Arc::new(HttpAccessLease::Local));}
    Err(AppError::Forbidden("voice activation requires its live authenticated lease".into()))
}

#[derive(Clone)]
pub(crate) struct VoiceRouterState {
    pub service:Arc<VoiceSessionService>,pub registry:Arc<AppVoiceRegistry>,pub authority:Arc<AppVoiceAuthority>,pub allowed_origins:Arc<[String]>,pub token_validator:Arc<nomifun_auth::InstanceTokenValidator>,pub jwt:Arc<nomifun_auth::JwtService>,pub shutdown:tokio_util::sync::CancellationToken,pub cleanup_registered:Arc<std::sync::atomic::AtomicBool>,
}
pub(crate) fn routes(state:VoiceRouterState)->Router{
    Router::new().route("/api/mobile-voice/v1/capabilities",get(capabilities)).route("/api/mobile-voice/v1/profiles/{id}",axum::routing::put(save_profile)).route("/api/mobile-voice/v1/profiles/{id}/probe",post(probe_profile)).route("/api/mobile-voice/v1/sessions/{id}/work",post(work_command)).route("/api/mobile-voice/v1/catalog",get(catalog)).route("/api/mobile-voice/v1/availability",get(availability))
        .route("/api/mobile-voice/v1/sessions",post(activate)).route("/api/mobile-voice/v1/sessions/{id}",get(session_state))
        .route("/api/mobile-voice/v1/sessions/{id}/projection",get(projection))
        .route("/api/mobile-voice/v1/sessions/{id}/source-context",post(confirm_source_context))
        .route("/api/mobile-voice/v1/sessions/{id}/control",post(control)).route("/api/mobile-voice/v1/sessions/{id}/attachment",post(rebuild))
        .route("/api/mobile-voice/v1/data/export",get(export_data)).route("/api/mobile-voice/v1/data/sessions/{session}",axum::routing::delete(delete_data)).with_state(state)
}
pub(crate) fn media_routes(state:VoiceRouterState)->Router{Router::new().route("/api/mobile-voice/v1/sessions/{id}/media",get(media)).with_state(state)}
fn error(error:VoiceError)->AppError{match error.kind{VoiceErrorKind::Authentication=>AppError::Forbidden(error.message),VoiceErrorKind::StaleBinding|VoiceErrorKind::StaleEpoch=>AppError::Conflict(error.message),_=>AppError::BadRequest(error.message)}}
async fn catalog(State(state):State<VoiceRouterState>)->Result<Json<ApiResponse<serde_json::Value>>,AppError>{Ok(Json(ApiResponse::ok(state.registry.catalog().await.map_err(error)?)))}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct AvailabilityQuery{agent_session_id:String}
async fn availability(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,Query(query):Query<AvailabilityQuery>)->Result<Json<ApiResponse<serde_json::Value>>,AppError>{
    let mut value=state.authority.availability(user.id.as_ref(),&query.agent_session_id).await.map_err(error)?;
    if let Some(active)=state.service.active_binding(user.id.as_ref(),&query.agent_session_id).await{value["active_voice"]=active;}
    Ok(Json(ApiResponse::ok(value)))
}
async fn activate(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,local:Option<Extension<nomifun_auth::trust::LocalTrusted>>,headers:HeaderMap,Json(request):Json<VoiceActivationRequest>)->Result<Json<ApiResponse<VoiceActivationResponse>>,AppError>{
    let access=auth_lease(&state,&headers,user.id.as_ref(),local.is_some())?;
    if state.cleanup_registered.compare_exchange(false,true,std::sync::atomic::Ordering::AcqRel,std::sync::atomic::Ordering::Acquire).is_ok(){
        let service=state.service.clone();let shutdown=state.shutdown.clone();let authority=state.authority.clone();let owner=user.id.as_ref().to_owned();
        if !state.authority.sessions.voice_register_task(Box::pin(async move{
            let mut interval=tokio::time::interval(Duration::from_secs(30));interval.tick().await;
            loop{tokio::select!{biased;_=shutdown.cancelled()=>break,_=interval.tick()=>{
                let Some(journal)=authority.journal.opened().await else{continue;};
                if let Ok(sessions)=journal.referenced_sessions(owner.clone()).await{for session in sessions{
                    if shutdown.is_cancelled(){break;}
                    if authority.sessions.canonical().store().inspect_tombstone(&session.clone().into()).await.is_ok_and(|tombstone|tombstone.is_some()){
                        if let Err(error)=service.delete_owned_data(&owner,&session,false).await{tracing::warn!(error_kind=?error.kind,"voice tombstone cleanup deferred");}
                    }
                }}
            }}}
            service.shutdown().await;
        })){
            state.cleanup_registered.store(false,std::sync::atomic::Ordering::Release);return Err(AppError::Conflict("application is closing; voice activation refused".into()));
        }
    }
    Ok(Json(ApiResponse::ok(state.service.activate_with_access(user.id.as_ref(),request,access).await.map_err(error)?)))
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct EpochQuery{activation_epoch:u64}
async fn session_state(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,Path(id):Path<String>,Query(query):Query<EpochQuery>)->Result<Json<ApiResponse<VoiceState>>,AppError>{
    Ok(Json(ApiResponse::ok(state.service.state(user.id.as_ref(),&id,query.activation_epoch).await.map_err(error)?)))
}
async fn projection(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,Path(id):Path<String>,Query(query):Query<EpochQuery>)->Result<Json<ApiResponse<VoiceSessionProjection>>,AppError>{
    Ok(Json(ApiResponse::ok(state.service.projection(user.id.as_ref(),&id,query.activation_epoch).await.map_err(error)?)))
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct ControlRequest{activation_epoch:u64,control:VoiceControl}
async fn control(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,Path(id):Path<String>,Json(request):Json<ControlRequest>)->Result<Json<ApiResponse<VoiceState>>,AppError>{
    Ok(Json(ApiResponse::ok(state.service.control(user.id.as_ref(),&id,request.activation_epoch,request.control).await.map_err(error)?)))
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct NativeOffer{activation_epoch:u64,native_offer:String}
async fn rebuild(State(state):State<VoiceRouterState>,Extension(user):Extension<CurrentUser>,Path(id):Path<String>,Json(request):Json<NativeOffer>)->Result<Json<ApiResponse<VoiceActivationResponse>>,AppError>{
    if request.native_offer.len()>128*1024{return Err(AppError::BadRequest("native SDP exceeds limit".into()));}
    Ok(Json(ApiResponse::ok(state.service.rebuild_native(user.id.as_ref(),&id,request.activation_epoch,request.native_offer).await.map_err(error)?)))
}
async fn media(State(state):State<VoiceRouterState>,Path(id):Path<String>,headers:HeaderMap,ws:WebSocketUpgrade)->Result<Response,AppError>{
    if !nomifun_realtime::validate_attachment_origin(&headers,&state.allowed_origins){return Err(AppError::Forbidden("voice media origin rejected".into()));}
    let ticket=headers.get("sec-websocket-protocol").and_then(|header|header.to_str().ok()).and_then(|value|value.split(',').map(str::trim).find_map(|value|value.strip_prefix("nomifun.voice.ticket."))).ok_or_else(||AppError::Forbidden("voice media attachment capability missing".into()))?;
    let owner=state.service.capability_owner(&id,ticket).await.map_err(error)?;
    let attachment=state.service.attach_media(&owner,&id,ticket).await.map_err(error)?;
    Ok(ws.protocols(["nomifun.voice.v1"]).max_message_size(512*1024+4096+8).max_frame_size(512*1024+4096+8).on_upgrade(move|socket|media_socket(socket,state.service,owner,id,attachment)))
}
#[derive(Serialize,Deserialize)]#[serde(deny_unknown_fields)]struct AudioMetadata{
    #[serde(default,skip_serializing_if="Option::is_none")]segment_id:Option<String>,
    activation_epoch:u64,output_generation:u64,sequence:u64,timestamp:u64,duration_us:u32,format:AudioFormat,
}
fn decode_audio(packet:&[u8])->Result<AudioFrame,AppError>{
    if packet.len()<8||&packet[..4]!=b"NFV1"{return Err(AppError::BadRequest("invalid voice media version".into()));}
    let length=u32::from_le_bytes(packet[4..8].try_into().unwrap())as usize;
    if length>4096||packet.len()<8+length||packet.len()-8-length>512*1024{return Err(AppError::BadRequest("voice media bounds exceeded".into()));}
    let metadata:AudioMetadata=serde_json::from_slice(&packet[8..8+length]).map_err(|_|AppError::BadRequest("invalid voice media metadata".into()))?;
    if metadata.segment_id.is_some(){return Err(AppError::BadRequest("uplink cannot mint assistant segments".into()));}
    Ok(AudioFrame{activation_epoch:metadata.activation_epoch,output_generation:metadata.output_generation,sequence:metadata.sequence,timestamp:metadata.timestamp,duration_us:metadata.duration_us,format:metadata.format,payload:packet[8+length..].to_vec()})
}
fn encode_audio(segment_id:String,frame:AudioFrame)->Result<Vec<u8>,AppError>{
    let metadata=serde_json::to_vec(&AudioMetadata{segment_id:Some(segment_id),activation_epoch:frame.activation_epoch,output_generation:frame.output_generation,sequence:frame.sequence,timestamp:frame.timestamp,duration_us:frame.duration_us,format:frame.format})
        .map_err(|_|AppError::BadRequest("voice media metadata encoding failed".into()))?;
    if metadata.len()>4096||frame.payload.len()>512*1024{return Err(AppError::BadRequest("voice output exceeds media bounds".into()));}
    let mut packet=Vec::with_capacity(8+metadata.len()+frame.payload.len());packet.extend_from_slice(b"NFV1");packet.extend_from_slice(&(metadata.len()as u32).to_le_bytes());packet.extend(metadata);packet.extend(frame.payload);Ok(packet)
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct ForegroundHeartbeat{kind:String,activation_epoch:u64,sequence:u64}
async fn media_socket(socket:WebSocket,service:Arc<VoiceSessionService>,owner:String,id:String,mut attachment:VoiceMediaAttachment){
    let epoch=attachment.state.activation_epoch;let(sink,mut stream)=socket.split();let cancel=tokio_util::sync::CancellationToken::new();
    let final_reason=Arc::new(std::sync::Mutex::new(VoiceCloseReason::NetworkLost));let writer_reason=final_reason.clone();
    let writer_cancel=cancel.clone();let writer_service=service.clone();let writer_owner=owner.clone();let writer_id=id.clone();
    let mut writer=tokio::spawn(async move{
        let mut sink=sink;
        let initial=VoiceProductEvent::State{state:attachment.state};
        if let Err(reason)=send_media_message(&mut sink,Message::Text(serde_json::to_string(&initial).unwrap_or_default().into())).await{*writer_reason.lock().unwrap_or_else(|p|p.into_inner())=reason;writer_cancel.cancel();return;}
        loop{tokio::select!{biased;
            _=writer_cancel.cancelled()=>break,
            changed=attachment.controls.changed()=>{
                if changed.is_err(){break;}
                attachment.controls.borrow_and_update();
                for event in attachment.endpoint.control_snapshot(){
                    let closed=matches!(&event,VoiceProductEvent::EndpointControl{control:VoiceControl::Close{..},..});
                    let encoded=serde_json::to_string(&event).unwrap_or_default();
                    if let Err(reason)=send_media_message(&mut sink,Message::Text(encoded.into())).await{*writer_reason.lock().unwrap_or_else(|p|p.into_inner())=reason;writer_cancel.cancel();break;}
                    if closed{writer_cancel.cancel();break;}
                }
            },
            event=attachment.events.recv()=>{
                let event=match event{Ok(event)=>event,Err(broadcast_error)=>{if matches!(broadcast_error,tokio::sync::broadcast::error::RecvError::Lagged(_)){*writer_reason.lock().unwrap_or_else(|p|p.into_inner())=VoiceCloseReason::Backlog;}break;}};
                let closed=matches!(&event,VoiceProductEvent::Model{event:VoiceModelEvent::Closed{..}});
                let encoded=serde_json::to_string(&event).unwrap_or_default();
                if encoded.len()>256*1024{break;}
                if let Err(reason)=send_media_message(&mut sink,Message::Text(encoded.into())).await{*writer_reason.lock().unwrap_or_else(|p|p.into_inner())=reason;break;}
                if closed{break;}
            },
            frame=attachment.frames.recv()=>{
                let Some(frame)=frame else{break;};
                if frame.age()>attachment.max_age{*writer_reason.lock().unwrap_or_else(|p|p.into_inner())=VoiceCloseReason::Backlog;break;}
                if !frame.is_current(){continue;}
                let current=writer_service.state(&writer_owner,&writer_id,epoch).await;
                if !current.is_ok_and(|s|s.output_generation==frame.frame.output_generation){continue;}
                let Ok(packet)=encode_audio(frame.segment_id,frame.frame)else{break;};
                if let Err(reason)=send_media_message(&mut sink,Message::Binary(packet.into())).await{*writer_reason.lock().unwrap_or_else(|p|p.into_inner())=reason;break;}
            }
        }}
        writer_cancel.cancel();let _=tokio::time::timeout(Duration::from_millis(200),sink.close()).await;
    });
    loop{tokio::select!{biased;
        _=cancel.cancelled()=>break,
        message=stream.next()=>{
            match message{
                Some(Ok(Message::Binary(packet)))=>{let Ok(frame)=decode_audio(&packet)else{break;};if service.audio(&owner,&id,frame).await.is_err(){break;}},
                Some(Ok(Message::Text(text)))=>{let Ok(heartbeat)=serde_json::from_str::<ForegroundHeartbeat>(&text)else{break;};if heartbeat.kind!="foreground"||service.foreground(&owner,&id,heartbeat.activation_epoch,heartbeat.sequence).await.is_err(){break;}},
                Some(Ok(Message::Ping(_)|Message::Pong(_)))=>{},
                Some(Ok(Message::Close(_)))|None=>break,_=>break,
            }
        }
    }}
    cancel.cancel();if tokio::time::timeout(Duration::from_secs(1),&mut writer).await.is_err(){writer.abort();let _=writer.await;}
    let reason=*final_reason.lock().unwrap_or_else(|p|p.into_inner());
    let _=service.control(&owner,&id,epoch,VoiceControl::Close{reason}).await;
}
async fn send_media_message(sink:&mut futures_util::stream::SplitSink<WebSocket,Message>,message:Message)->Result<(),VoiceCloseReason>{
    match tokio::time::timeout(Duration::from_millis(200),sink.send(message)).await{
        Ok(Ok(()))=>Ok(()),Ok(Err(_))=>Err(VoiceCloseReason::NetworkLost),Err(_)=>Err(VoiceCloseReason::Backlog)
    }
}

#[cfg(test)]mod tests{use super::*;
    #[test]fn media_version_and_explicit_format_are_roundtrip_and_bounded(){
        let frame=AudioFrame{activation_epoch:2,output_generation:4,sequence:1,timestamp:20_000,duration_us:20_000,format:AudioFormat::pcm16(48_000,2),payload:vec![0;3840]};
        let packet=encode_audio("segment".into(),frame.clone()).unwrap();assert!(decode_audio(&packet).is_err(),"uplink rejects output identity");
        let mut metadata:serde_json::Value=serde_json::from_slice(&packet[8..8+u32::from_le_bytes(packet[4..8].try_into().unwrap())as usize]).unwrap();metadata.as_object_mut().unwrap().remove("segment_id");
        let header=serde_json::to_vec(&metadata).unwrap();let mut uplink=b"NFV1".to_vec();uplink.extend_from_slice(&(header.len()as u32).to_le_bytes());uplink.extend(header);uplink.extend(frame.payload.clone());
        assert_eq!(decode_audio(&uplink).unwrap(),frame);assert!(decode_audio(b"old").is_err());
        let mut oversized=b"NFV1".to_vec();oversized.extend_from_slice(&5000u32.to_le_bytes());assert!(decode_audio(&oversized).is_err());
    }
}
