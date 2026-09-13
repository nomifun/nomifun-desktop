//! Configure the isolated product-test model. Read its credential from stdin;
//! never pass it in argv, print it, or persist plaintext credential files.
use nomifun_db::{
    IClientPreferenceRepository, SqliteClientPreferenceRepository,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository,
};
use std::{path::PathBuf, sync::Arc};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("An isolated data directory is required"))?,
    );
    if !root.join("nomifun-backend.db").is_file() || !root.join("encryption_key").is_file() {
        anyhow::bail!("Start the isolated product instance first");
    }
    let mut credential = zeroize::Zeroizing::new(String::new());
    std::io::stdin().read_line(&mut credential)?;
    if credential.trim().is_empty() {
        anyhow::bail!("A credential is required on stdin");
    }
    let raw = zeroize::Zeroizing::new(std::fs::read_to_string(root.join("encryption_key"))?);
    if raw.trim().len() != 64 {
        anyhow::bail!("Invalid instance encryption key");
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&raw.trim()[index * 2..index * 2 + 2], 16)?;
    }
    let db = nomifun_db::init_database(&root.join("nomifun-backend.db")).await?;
    let service = nomifun_system::ProviderService::new(
        Arc::new(SqliteProviderRepository::new(db.pool().clone())),
        Arc::new(SqliteProviderModelRepository::new(db.pool().clone())),
        Arc::new(SqliteProviderModelCapabilityRepository::new(
            db.pool().clone(),
        )),
        Arc::new(SqliteProviderConnectionRepository::new(db.pool().clone())),
        key,
    );
    let request = serde_json::from_value(serde_json::json!({
        "platform":"stepfun-plan","name":"StepFun Coding Plan · Plugin validation",
        "base_url":"https://api.stepfun.com/step_plan/v1","auth_scheme":"bearer",
        "credentials":{"api_keys":[credential.trim()]},"enabled":true,
        "initial_model":{"model":"step-3.7-flash","capabilities":[{"task":"chat","traits":["function_calling","streaming"],"protocol":"openai.chat_text","connection_role":"default","provider_params":{}}]}
    }))?;
    let provider = service
        .create(request)
        .await
        .map_err(|_| anyhow::anyhow!("Unable to configure the paid test provider"))?;
    let preference =
        serde_json::json!({"provider_id":provider.provider_id,"model":"step-3.7-flash"})
            .to_string();
    SqliteClientPreferenceRepository::new(db.pool().clone())
        .upsert_batch(&[("nomi.defaultModel", &preference)])
        .await?;
    println!("Configured StepFun Coding Plan / step-3.7-flash; credential stored encrypted.");
    db.close().await;
    Ok(())
}
