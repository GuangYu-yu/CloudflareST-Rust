use hyper::{Client, Body};
use hyper_tls::HttpsConnector;
use std::env;
use std::time::Duration;
use tokio::time::timeout;

pub async fn check_update() -> Option<String> {
    let https = HttpsConnector::new();
    let client = Client::builder()
        .build::<_, Body>(https);
    
    // 使用 tokio 的 timeout 包装整个请求过程    
    let fut = async {
        let res = client
            .get("https://ver.797874.xyz".parse().ok()?)
            .await
            .ok()?;
            
        let bytes = hyper::body::to_bytes(res.into_body())
            .await
            .ok()?;
            
        String::from_utf8(bytes.to_vec()).ok()
    };
    
    // 应用10秒超时
    let body = timeout(Duration::from_secs(10), fut)
        .await
        .ok()??;
    
    let current_version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    if body != current_version {
        Some(body)
    } else {
        None
    }
} 