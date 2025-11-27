use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use http_body_util::{Full, BodyExt};
use hyper::{body::Bytes, Method, Request, Response, Uri, body::Incoming};
use hyper::header::{HeaderValue, CONNECTION};
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::{TokioIo};
use hyper_rustls::HttpsConnectorBuilder;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tower::Service;
use url::Url;

use crate::interface::{InterfaceIps, bind_socket_to_interface};

/// 浏览器 User-Agent
pub const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// 自定义 Connector，支持绑定网卡
#[derive(Clone)]
pub struct InterfaceConnector {
    addr: SocketAddr,
    interface: Option<String>,
    interface_ips: Option<InterfaceIps>,
    timeout: Duration,
}

impl Service<Uri> for InterfaceConnector {
    type Response = TokioIo<TcpStream>;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _uri: Uri) -> Self::Future {
        let interface = self.interface.clone();
        let interface_ips = self.interface_ips.clone();
        let timeout_duration = self.timeout;
        let addr = self.addr;

        Box::pin(async move {
            // 尝试绑定到指定网卡
            if let Some(socket) = bind_socket_to_interface(addr, interface.as_deref(), interface_ips.as_ref()).await {
                let stream = timeout(timeout_duration, socket.connect(addr)).await??;
                return Ok(TokioIo::new(stream));
            }
            
            // 绑定失败，直接返回错误
            Err("".into())
        })
    }
}

/// 创建基础的HTTP客户端构建器
pub fn client_builder() -> Result<Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .build();
    
    Ok(Client::builder(hyper_util::rt::TokioExecutor::new()).build(https_connector))
}

/// 构建 hyper 客户端
pub fn build_hyper_client(
    addr: SocketAddr,
    interface: Option<&str>,
    interface_ips: Option<&InterfaceIps>,
    timeout_ms: u64,
) -> Option<Client<hyper_rustls::HttpsConnector<InterfaceConnector>, Full<Bytes>>> {
    let connector = InterfaceConnector {
        addr,
        interface: interface.map(|s| s.to_string()),
        interface_ips: interface_ips.cloned(),
        timeout: Duration::from_millis(timeout_ms),
    };

    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .wrap_connector(connector);

    Some(Client::builder(hyper_util::rt::TokioExecutor::new()).build(https_connector))
}

// 简单 GET 请求
pub async fn send_get_request_simple(
    client: &mut Client<hyper_rustls::HttpsConnector<HttpConnector>, Full<Bytes>>,
    host: &str,
    uri: Uri,
    timeout_ms: u64,
) -> Result<Bytes, Box<dyn std::error::Error + Send + Sync>> {
    let req = Request::builder()
        .uri(uri)
        .method(Method::GET)
        .header("User-Agent", USER_AGENT)
        .header("Host", host)
        .body(Full::new(Bytes::new()))?;

    let resp = timeout(Duration::from_millis(timeout_ms), client.call(req)).await??;
    
    let body_bytes = resp.into_body().collect().await?.to_bytes();
    
    Ok(body_bytes)
}

/// 发送 GET 请求并返回流式响应
pub async fn send_get_response(
    client: &Client<hyper_rustls::HttpsConnector<InterfaceConnector>, Full<Bytes>>,
    host: &str,
    uri: Uri,
    timeout_ms: u64,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    let req = Request::builder()
        .uri(uri)
        .method(Method::GET)
        .header("User-Agent", USER_AGENT)
        .header("Host", host)
        .body(Full::new(Bytes::from(vec![])))?;

    let resp = timeout(Duration::from_millis(timeout_ms), client.request(req)).await??;
    Ok(resp)
}

/// 发送 HEAD 请求
pub async fn send_head_request(
    client: &Client<hyper_rustls::HttpsConnector<InterfaceConnector>, Full<Bytes>>,
    host: &str,
    uri: Uri,
    timeout_ms: u64,
    close_connection: bool,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    let mut req_builder = Request::builder()
        .uri(uri)
        .method(Method::HEAD)
        .header("User-Agent", USER_AGENT)
        .header("Host", host);

    if close_connection {
        // 告诉服务器和客户端：这次请求后关闭连接 
        req_builder = req_builder.header(CONNECTION, HeaderValue::from_static("close"));
    }

    let req = req_builder
        .body(Full::new(Bytes::from(vec![])))?;

    let resp = timeout(Duration::from_millis(timeout_ms), client.request(req)).await??;
    Ok(resp)
}

/// 统一的URI解析函数，将URL字符串解析为hyper::Uri
/// 这个函数封装了URL解析和转换的通用逻辑，避免在多个模块中重复
pub fn parse_url_to_uri(url_str: &str) -> Option<(Uri, String)> {
    // 使用url库解析URL
    let url_parts = match Url::parse(url_str) {
        Ok(parts) => parts,
        Err(_) => return None,
    };

    // 提取主机名
    let host = match url_parts.host_str() {
        Some(host) => host.to_string(),
        None => return None,
    };

    // 将URL转换为hyper::Uri
    let uri = match Uri::try_from(url_parts.as_str()) {
        Ok(uri) => uri,
        Err(_) => return None,
    };

    Some((uri, host))
}