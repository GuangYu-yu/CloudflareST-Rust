use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use http::{Method, Uri};
use hyper::{Request, Response, body::Incoming};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::interface::{InterfaceParamResult, bind_socket_to_interface};

pub(crate) const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

// TCP/TLS 流的动态分发 trait
pub(crate) trait IoBox: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> IoBox for T {}

// TLS（跳过证书验证）
#[derive(Debug)]
struct NoCertVerifier;

impl rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}

pub(crate) fn build_tls_connector() -> std::io::Result<Arc<TlsConnector>> {
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoCertVerifier))
        .with_no_client_auth();
    Ok(Arc::new(TlsConnector::from(Arc::new(config))))
}

/// 请求上下文，包装重复的配置参数
#[derive(Clone)]
pub(crate) struct RequestContext {
    pub interface_config: Arc<InterfaceParamResult>,
    pub tls_connector: Arc<TlsConnector>,
    pub connect_timeout_ms: u64,
    pub ttfb_timeout_ms: u64,
}

impl RequestContext {
    /// 单次请求（不复用连接），用于下载测速
    pub(crate) async fn send_request(
        &self,
        host: &str,
        uri: &Uri,
        addr: SocketAddr,
        method: &Method,
    ) -> Option<Response<Incoming>> {
        let mut conn = self.open(host, uri, addr).await?;

        timeout(
            Duration::from_millis(self.ttfb_timeout_ms),
            conn.request(uri, method),
        )
        .await
        .ok()?
        .ok()
    }

    /// 建立可复用连接（延迟测速同一 IP 多次请求复用）
    pub(crate) async fn open(
        &self,
        host: &str,
        uri: &Uri,
        addr: SocketAddr,
    ) -> Option<HttpConnection> {
        connect(
            &self.interface_config,
            &self.tls_connector,
            host,
            uri,
            addr,
            self.connect_timeout_ms,
        ).await
    }
}

pub(crate) struct EmptyBody;

impl hyper::body::Body for EmptyBody {
    type Data = &'static [u8];
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        std::task::Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        true
    }
}

/// 可复用的 HTTP/1.1 连接：sender 发送请求，_task 后台驱动连接状态机
pub(crate) struct HttpConnection {
    sender: hyper::client::conn::http1::SendRequest<EmptyBody>,
    host_header: String,
    _task: tokio::task::JoinHandle<()>,
}

impl HttpConnection {
    pub(crate) async fn request(
        &mut self,
        uri: &Uri,
        method: &Method,
    ) -> Result<Response<Incoming>, hyper::Error> {
        self.sender.ready().await?;

        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("Host", &self.host_header)
            .header("User-Agent", USER_AGENT)
            .body(EmptyBody)
            .unwrap();

        self.sender.send_request(req).await
    }
}

pub(crate) async fn connect(
    interface_config: &Arc<InterfaceParamResult>,
    tls_connector: &TlsConnector,
    host: &str,
    uri: &Uri,
    addr: SocketAddr,
    timeout_ms: u64,
) -> Option<HttpConnection> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);

    let socket = bind_socket_to_interface(addr, interface_config).await?;
    let remaining = deadline.checked_duration_since(Instant::now())?;
    let stream = timeout(remaining, socket.connect(addr)).await.ok()?.ok()?;
    stream.set_nodelay(true).ok();

    // TLS 握手（仅 HTTPS）
    let stream: Box<dyn IoBox> = if uri.scheme_str() == Some("https") {
        let server_name = rustls_pki_types::ServerName::try_from(host.to_string()).unwrap();
        let remaining = deadline.checked_duration_since(Instant::now())?;
        let tls_stream = timeout(remaining, tls_connector.connect(server_name, stream))
            .await.ok()?.ok()?;
        Box::new(tls_stream)
    } else {
        Box::new(stream)
    };

    // HTTP/1.1 握手
    let io = TokioIo::new(stream);
    let (sender, conn) = hyper::client::conn::http1::Builder::new()
        .handshake(io)
        .await.ok()?;

    // 后台驱动 HTTP 连接状态机（不 poll conn 会停滞）
    let _task = tokio::spawn(async move {
        let _ = conn.await;
    });

    let host_header = format!("{host}:{}", addr.port());
    Some(HttpConnection { sender, host_header, _task })
}

pub(crate) fn parse_url_to_uri(url_str: &str) -> Option<(Uri, String)> {
    let uri = url_str.parse::<Uri>().ok()?;
    let host = uri.host()?.to_string();
    Some((uri, host))
}