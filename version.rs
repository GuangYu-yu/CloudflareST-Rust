use reqwest::Client;
use std::time::Duration;

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
    
    if body != env!("CARGO_PKG_VERSION") {
        Some(body)
    } else {
        None
    }
} 