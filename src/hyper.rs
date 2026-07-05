use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http::{Method, Uri};
use hyper::{Request, Response, body::Incoming};
use hyper_util::rt::TokioIo;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::interface::{InterfaceParamResult, bind_socket_to_interface};

pub(crate) const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

// TLS（跳过证书验证）

#[derive(Debug)]
struct NoCertVerifier;

impl rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
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

// 空的请求体

pub(crate) struct EmptyBody;

impl hyper::body::Body for EmptyBody {
    type Data = &'static [u8];
    type Error = std::convert::Infallible;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<hyper::body::Frame<Self::Data>, Self::Error>>> {
        std::task::Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        true
    }
}

// 发送 HTTP 请求

pub(crate) async fn send_request(
    interface_config: &Arc<InterfaceParamResult>,
    tls_connector: &TlsConnector,
    host: &str,
    uri: &Uri,
    addr: SocketAddr,
    method: &Method,
    connect_timeout_ms: u64,
    ttfb_timeout_ms: u64,
) -> Option<Response<Incoming>> {
    // TCP 连接
    let socket = bind_socket_to_interface(addr, interface_config).await?;
    let stream = timeout(Duration::from_millis(connect_timeout_ms), socket.connect(addr))
        .await
        .ok()?
        .ok()?;
    stream.set_nodelay(true).ok();

    // TLS 握手
    let server_name = rustls_pki_types::ServerName::try_from(host.to_string()).ok()?;
    let tls_stream = timeout(
        Duration::from_millis(connect_timeout_ms),
        tls_connector.connect(server_name, stream),
    )
    .await
    .ok()?
    .ok()?;

    // HTTP/1.1 握手
    let io = TokioIo::new(tls_stream);
    let (mut sender, conn) = hyper::client::conn::http1::Builder::new()
        .handshake(io)
        .await
        .ok()?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("[hyper] connection error: {e}");
        }
    });

    // 构造请求（Host 头带端口号，兼容非默认端口）
    let host_header = match uri.port_u16() {
        Some(port) if port != 443 && port != 80 => format!("{host}:{port}"),
        _ => host.to_string(),
    };

    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("Host", &host_header)
        .header("User-Agent", USER_AGENT)
        .body(EmptyBody)
        .ok()?;

    // 发送请求并等待首字节
    let resp = timeout(Duration::from_millis(ttfb_timeout_ms), sender.send_request(req))
        .await
        .ok()?
        .ok()?;

    Some(resp)
}

pub(crate) fn parse_url_to_uri(url_str: &str) -> Option<(Uri, String)> {
    let uri = url_str.parse::<Uri>().ok()?;
    let host = uri.host()?.to_string();
    Some((uri, host))
}