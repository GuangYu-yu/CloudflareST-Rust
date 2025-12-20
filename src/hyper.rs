use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use std::sync::Arc;

use http_body_util::Full;
use hyper::{body::Bytes, Method, Request, Response, Uri, body::Incoming};
use hyper::header::{HeaderValue, CONNECTION};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioIo;
use hyper_rustls::HttpsConnectorBuilder;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tower_service::Service;

use crate::interface::{InterfaceParamResult, bind_socket_to_interface};

/// 浏览器 User-Agent
pub(crate) const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// 自定义 Connector，支持绑定网卡
#[derive(Clone)]
pub(crate) struct InterfaceConnector {
    addr: SocketAddr,
    interface_config: Arc<InterfaceParamResult>,
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
        let interface_config = Arc::clone(&self.interface_config);
        let timeout_duration = self.timeout;
        let addr = self.addr;

        Box::pin(async move {
            let socket = bind_socket_to_interface(addr, &interface_config).await.unwrap();
            let stream = timeout(timeout_duration, socket.connect(addr)).await??;
            Ok(TokioIo::new(stream))
        })
    }
}

/// 构建 hyper 客户端
pub(crate) fn build_hyper_client(
    addr: SocketAddr,
    interface_config: &Arc<InterfaceParamResult>,
    timeout_ms: u64,
) -> Option<Client<hyper_rustls::HttpsConnector<InterfaceConnector>, Full<Bytes>>> {
    let connector = InterfaceConnector {
        addr,
        interface_config: Arc::clone(interface_config),
        timeout: Duration::from_millis(timeout_ms),
    };

    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .enable_http1()
        .wrap_connector(connector);

    Some(Client::builder(hyper_util::rt::TokioExecutor::new()).build(https_connector))
}

/// 发送 GET 请求并返回流式响应
pub(crate) async fn send_get_response(
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
        .body(Full::new(Bytes::new()))?;

    let resp = timeout(Duration::from_millis(timeout_ms), client.request(req)).await??;
    Ok(resp)
}

/// 发送 HEAD 请求
pub(crate) async fn send_head_request(
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
        .body(Full::new(Bytes::new()))?;

    let resp = timeout(Duration::from_millis(timeout_ms), client.request(req)).await??;
    Ok(resp)
}

/// 统一的URI解析函数
pub(crate) fn parse_url_to_uri(url_str: &str) -> Option<(Uri, String)> {
    // 1. 尝试解析为 Uri
    let uri = url_str.parse::<Uri>().ok()?;

    // 2. 提取主机名 (host)
    let host = uri.host()?.to_string();

    Some((uri, host))
}