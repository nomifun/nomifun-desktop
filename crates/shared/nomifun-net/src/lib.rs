pub mod proxy;

pub fn http_client() -> reqwest::Client {
    proxy::apply_detected_proxy(reqwest::Client::builder())
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}
