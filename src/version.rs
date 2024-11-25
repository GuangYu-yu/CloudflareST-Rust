use reqwest::Client;
use std::time::Duration;
use std::env;

pub async fn check_update() -> Option<String> {
    let timeout = Duration::from_secs(10);
    let client = Client::builder()
        .timeout(timeout)
        .build()
        .ok()?;
        
    let res = client
        .get("https://ver.797874.xyz")
        .send()
        .await
        .ok()?;
        
    let body = res.text().await.ok()?;
    
    let current_version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    if body != current_version {
        Some(body)
    } else {
        None
    }
} 