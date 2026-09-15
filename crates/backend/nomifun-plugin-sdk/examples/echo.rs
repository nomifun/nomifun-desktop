use nomifun_plugin_sdk::{CallContext, Service, async_trait};
use serde_json::{Value, json};

struct Echo;

#[async_trait]
impl Service for Echo {
    async fn invoke(
        &self,
        method: &str,
        payload: Value,
        context: &mut CallContext,
    ) -> Result<Value, String> {
        match method {
            "echo" | "plugin.callable.echo.invoke" | "plugin.rust-echo.invoke" => {
                Ok(json!({"method":method, "payload":payload, "call_id":context.call_id}))
            }
            "stream" => {
                for sequence in 1..=3 {
                    context.emit(json!({"sequence":sequence})).await?;
                }
                Ok(Value::Null)
            }
            "kv" => context.storage("kv", payload).await,
            "hang" => std::future::pending().await,
            "crash" => std::process::exit(86),
            _ => Err(format!("unknown method: {method}")),
        }
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    nomifun_plugin_sdk::serve(Echo).await
}
