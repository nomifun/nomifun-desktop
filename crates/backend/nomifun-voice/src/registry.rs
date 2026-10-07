use std::collections::BTreeMap;
use std::sync::Arc;
use std::future::Future;
use std::pin::Pin;
use nomifun_voice_contracts::voice::*;
use nomifun_voice_core::VoiceModelPort;
use serde_json::Value;

/// Factory closure captures the application-resolved credential lease. Secrets never enter core types.
pub type VoicePortFactory = Arc<dyn Fn(VoiceRouteRecord) -> Pin<Box<dyn Future<Output = Result<Arc<dyn VoiceModelPort>, VoiceError>> + Send>> + Send + Sync>;
pub type VoiceConfigValidator = Arc<dyn Fn(&Value) -> Result<(), VoiceError> + Send + Sync>;
pub type VoiceDescriptorFactory = Arc<dyn Fn(&str) -> VoiceAdapterDescriptor + Send + Sync>;
pub struct VoiceAdapterRegistration {
    pub adapter_id: String,
    pub contract_version: u32,
    pub config_schema: Value,
    /// Read-only product catalog projection owned by this exact registration.
    pub catalog_projection: Value,
    pub describe: VoiceDescriptorFactory,
    pub validate: VoiceConfigValidator,
    pub factory: VoicePortFactory,
}
pub struct VoiceAdapterRegistry { registrations: BTreeMap<String, VoiceAdapterRegistration> }
impl VoiceAdapterRegistry {
    pub fn new(registrations: Vec<VoiceAdapterRegistration>) -> Result<Self, VoiceError> {
        let mut map = BTreeMap::new();
        for registration in registrations {
            if registration.adapter_id.trim().is_empty() || !registration.adapter_id.contains('.')
                || registration.contract_version != VOICE_CONTRACT_VERSION || !registration.config_schema.is_object() {
                return Err(VoiceError::new(VoiceErrorKind::Configuration, "invalid voice adapter registration"));
            }
            if map.insert(registration.adapter_id.clone(), registration).is_some() {
                return Err(VoiceError::new(VoiceErrorKind::Configuration, "duplicate voice adapter registration"));
            }
        }
        Ok(Self { registrations: map })
    }
    pub fn registration(&self, id: &str) -> Result<&VoiceAdapterRegistration, VoiceError> {
        self.registrations.get(id).ok_or_else(|| VoiceError::new(VoiceErrorKind::Unsupported, format!("voice protocol {id:?} is not registered")))
    }
    pub fn entries(&self) -> impl Iterator<Item = &VoiceAdapterRegistration> { self.registrations.values() }
    pub fn validate_route(&self, record: &VoiceRouteRecord) -> Result<VoiceAdapterDescriptor, VoiceError> {
        record.validate().map_err(|e| VoiceError::new(VoiceErrorKind::Configuration, e))?;
        let registration = self.registration(&record.adapter_id)?;
        if record.adapter_contract_version != registration.contract_version { return Err(VoiceError::new(VoiceErrorKind::Configuration, "voice adapter contract changed")); }
        (registration.validate)(&record.adapter_config)?;
        let descriptor = (registration.describe)(&record.model);
        if descriptor.adapter_id != record.adapter_id || descriptor.contract_version != registration.contract_version
            || !descriptor.transports.contains(&record.transport) { return Err(VoiceError::new(VoiceErrorKind::Unsupported, "voice transport or descriptor identity mismatch")); }
        validate_native_duplex(&descriptor.capabilities).map_err(|e| VoiceError::new(VoiceErrorKind::Unsupported, e))?;
        for feature in &record.required_features {
            if !descriptor.capabilities.get(feature).is_some_and(|e| e.support == CapabilitySupport::Supported) {
                return Err(VoiceError::new(VoiceErrorKind::Unsupported, format!("voice capability {feature:?} is not verified")));
            }
        }
        Ok(descriptor)
    }
    pub async fn create(&self, record: &VoiceRouteRecord) -> Result<Arc<dyn VoiceModelPort>, VoiceError> {
        self.validate_route(record)?;
        (self.registration(&record.adapter_id)?.factory)(record.clone()).await
    }
    /// Explicit diagnostic through this registration's validator, factory and
    /// production port. Never invoked by catalog/profile saves/startup.
    pub async fn probe(&self,record:&VoiceRouteRecord,request:nomifun_voice_core::VoiceOpenRequest,cancel:tokio_util::sync::CancellationToken,deadline:tokio::time::Instant)->Result<(VoiceNegotiation,VoiceTermination),VoiceError>{
        self.validate_route(record)?;
        if request.transport!=record.transport||request.route_identity!=record.identity().map_err(|e|VoiceError::new(VoiceErrorKind::Configuration,e))?{
            return Err(VoiceError::new(VoiceErrorKind::StaleBinding,"probe must use the exact saved route and transport"));
        }
        if record.transport==VoiceTransportPreference::NativeWebrtc&&request.native_offer.is_none(){return Err(VoiceError::new(VoiceErrorKind::Unsupported,"native probe requires an actual endpoint SDP offer"));}
        let mut core=nomifun_voice_core::VoiceSessionCore::new(request.voice_session_id.clone(),request.replay_scope.agent_session_id.as_ref().into(),request.replay_scope.binding_version,request.activation_epoch);
        let opening=async{let model=self.create(record).await?;model.open(request,cancel.clone(),deadline).await};
        let session=match tokio::time::timeout_at(deadline,opening).await{Ok(result)=>result?,Err(_)=>{cancel.cancel();return Err(VoiceError::new(VoiceErrorKind::Deadline,"voice diagnostic open deadline"));}};
        let negotiation=session.negotiation.clone();
        if let Err(error)=core.negotiate(&negotiation){session.shutdown(VoiceCloseReason::ProviderFailed).await;return Err(error);}
        if negotiation.native_attachment.is_some()!=(record.transport==VoiceTransportPreference::NativeWebrtc){session.shutdown(VoiceCloseReason::ProviderFailed).await;return Err(VoiceError::new(VoiceErrorKind::Unsupported,"probe negotiated a different transport"));}
        let termination=session.shutdown(VoiceCloseReason::UserEnded).await;
        Ok((negotiation,termination))
    }
}
