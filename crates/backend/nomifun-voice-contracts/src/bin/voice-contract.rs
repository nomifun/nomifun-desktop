use std::collections::BTreeMap;
use std::path::PathBuf;
use nomifun_voice_contracts::*;
use schemars::{JsonSchema,schema_for};
fn add<T:JsonSchema>(map:&mut BTreeMap<&'static str,serde_json::Value>,name:&'static str){map.insert(name,serde_json::to_value(schema_for!(T)).expect("voice schema"));}
fn main()->Result<(),Box<dyn std::error::Error>>{
    let mode=std::env::args().nth(1).unwrap_or_else(||"check".into());
    let mut schemas=BTreeMap::new();
    add::<VoiceProfile>(&mut schemas,"voice_profile");add::<VoiceProfileUpdate>(&mut schemas,"voice_profile_update");
    add::<VoiceActivationRequest>(&mut schemas,"voice_activation_request");add::<VoiceActivationResponse>(&mut schemas,"voice_activation_response");
    add::<VoiceWorkCommand>(&mut schemas,"voice_work_command");add::<VoiceRouteRecord>(&mut schemas,"voice_route_record");
    add::<VoiceSourceContextConfirmation>(&mut schemas,"voice_source_context_confirmation");
    add::<VoiceSourceContextRequirement>(&mut schemas,"voice_source_context_requirement");
    add::<VoiceProfileProbeRequest>(&mut schemas,"voice_profile_probe_request");add::<VoiceProfileProbeResult>(&mut schemas,"voice_profile_probe_result");
    add::<VoiceRouteIdentity>(&mut schemas,"voice_route_identity");add::<VoiceAdapterDescriptor>(&mut schemas,"voice_adapter_descriptor");
    add::<VoiceModelEvent>(&mut schemas,"voice_model_event");add::<VoiceState>(&mut schemas,"voice_state");
    add::<VoiceControl>(&mut schemas,"voice_control");add::<VoiceNegotiation>(&mut schemas,"voice_negotiation");
    add::<VoiceProductEvent>(&mut schemas,"voice_product_event");add::<VoiceWorkReceipt>(&mut schemas,"voice_work_receipt");
    add::<VerifiedVoiceFact>(&mut schemas,"verified_voice_fact");add::<PlaybackReceipt>(&mut schemas,"playback_receipt");
    add::<VoiceReplayScope>(&mut schemas,"voice_replay_scope");add::<VoiceLocalFactRef>(&mut schemas,"voice_local_fact_ref");
    add::<VoiceLocalReplayEntry>(&mut schemas,"voice_local_replay_entry");add::<VoiceLocalReplay>(&mut schemas,"voice_local_replay");
    let output=serde_json::to_string_pretty(&schemas)?+"\n";
    let path=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contracts/schemas.json");
    match mode.as_str(){"write"=>{std::fs::create_dir_all(path.parent().unwrap())?;std::fs::write(path,output)?;},"check"=>{if std::fs::read_to_string(&path)?!=output{return Err("optional voice schemas are stale".into());}},_=>return Err("usage: voice-contract [write|check]".into())}
    Ok(())
}
