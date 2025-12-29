use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use rustls_pki_types::ServerName;
use hyper::{Method, Request, Response, Uri, body::Incoming};
use hyper_util::client::legacy::Client as LegacyClient;
use hyper_rustls::FixedServerNameResolver;
use hyper_util::rt::TokioIo;
use hyper_rustls::HttpsConnectorBuilder;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tower_service::Service;

use crate::interface::{InterfaceParamResult, bind_socket_to_interface};

/// 空的请求体实现
pub(crate) struct EmptyBody;

impl hyper::body::Body for EmptyBody {
    type Data = &'static [u8];
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        true
    }
}

#[derive(Clone)]
pub(crate) struct ConnectorService {
    interface_config: Arc<InterfaceParamResult>,
    timeout_duration: Duration,
}

impl ConnectorService {
    pub(crate) fn new(interface_config: Arc<InterfaceParamResult>, timeout_ms: u64) -> Self {
        Self {
            interface_config,
            timeout_duration: Duration::from_millis(timeout_ms),
        }
    }
}

impl Service<Uri> for ConnectorService {
    type Response = TokioIo<TcpStream>;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, uri: Uri) -> Self::Future {
        let config = Arc::clone(&self.interface_config);
        let t_duration = self.timeout_duration;

        Box::pin(async move {
            let addr: SocketAddr = format!("{}:{}", uri.host().unwrap(), uri.port_u16().unwrap())
                .parse()
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            let socket = bind_socket_to_interface(addr, &config)
                .await
                .unwrap_or_else(|| {
                    crate::error_and_exit(format_args!("绑定套接字到网络接口失败"));
                });
            
            let stream = timeout(t_duration, socket.connect(addr))
                .await
                .map_err(|_| "")? // 连接超时
                .map_err(|_| "")?; // 连接失败
            
            stream.set_nodelay(true).ok();
            Ok(TokioIo::new(stream))
        })
    }
}

pub(crate) type MyHttpsConnector = hyper_rustls::HttpsConnector<ConnectorService>;
pub(crate) type MyHyperClient = LegacyClient<MyHttpsConnector, EmptyBody>;

/// 浏览器 User-Agent
pub(crate) const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// 构建 hyper 客户端
pub(crate) fn build_hyper_client(
    interface_config: &Arc<InterfaceParamResult>,
    timeout_ms: u64,
    server_name: String,
) -> Option<MyHyperClient> {
    let connector = ConnectorService::new(Arc::clone(interface_config), timeout_ms);

    let resolver = FixedServerNameResolver::new(
        ServerName::try_from(server_name).ok()?
    );

    let https_connector = HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_or_http()
        .with_server_name_resolver(resolver)
        .enable_http1()
        .wrap_connector(connector);

    let client = LegacyClient::builder(hyper_util::rt::TokioExecutor::new())
        .pool_max_idle_per_host(1)
        .pool_idle_timeout(Duration::from_secs(1))
        .build(https_connector);

    Some(client)
}

/// 发送 HTTP 请求
pub(crate) async fn send_request(
    client: &MyHyperClient,
    host: &str,
    uri: Uri,
    method: Method,
    timeout_ms: u64,
) -> Result<Response<Incoming>, Box<dyn std::error::Error + Send + Sync>> {
    let req = Request::builder()
        .uri(uri)
        .method(method)
        .header("User-Agent", USER_AGENT)
        .header("Host", host)
        .body(EmptyBody)?;

    let resp = timeout(Duration::from_millis(timeout_ms), client.request(req)).await??;
    Ok(resp)
}

/// 统一的 URI 解析函数
pub(crate) fn parse_url_to_uri(url_str: &str) -> Option<(Uri, String)> {
    let uri = url_str.parse::<Uri>().ok()?;
    let host = uri.host()?.to_string();
    Some((uri, host))
}