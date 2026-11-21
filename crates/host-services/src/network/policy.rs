//! Exact origin normalization, manifest allowlist authorization, and connect-time IP enforcement.
use std::{
    collections::BTreeSet,
    fmt,
    io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    pin::Pin,
    sync::{Arc, OnceLock},
    task::{Context, Poll},
    time::Duration,
};
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpStream, lookup_host},
    time::timeout,
};
use tokio_rustls::{TlsConnector, client::TlsStream};
use url::{Host, Url};
use super::NetworkError;

/// Supported network schemes sealed in manifest origins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OriginScheme {
    Http,
    Https,
    Ws,
    Wss,
}

impl OriginScheme {
    /// Default port for this scheme.
    #[must_use]
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Http | Self::Ws => 80,
            Self::Https | Self::Wss => 443,
        }
    }

    /// Canonical lowercase scheme string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Ws => "ws",
            Self::Wss => "wss",
        }
    }

    /// Whether this scheme uses TLS transport.
    #[must_use]
    pub const fn is_tls(self) -> bool {
        matches!(self, Self::Https | Self::Wss)
    }

    /// Whether this scheme is HTTP or HTTPS.
    #[must_use]
    pub const fn is_http(self) -> bool {
        matches!(self, Self::Http | Self::Https)
    }

    /// Whether this scheme is WS or WSS.
    #[must_use]
    pub const fn is_ws(self) -> bool {
        matches!(self, Self::Ws | Self::Wss)
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "http" => Some(Self::Http),
            "https" => Some(Self::Https),
            "ws" => Some(Self::Ws),
            "wss" => Some(Self::Wss),
            _ => None,
        }
    }
}

/// Normalized exact origin `(scheme, host, effective_port)`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Origin {
    scheme: OriginScheme,
    host: String,
    port: u16,
}

impl Origin {
    /// Parses a manifest origin string (no path, query, fragment, userinfo, or wildcard).
    pub fn parse_manifest_entry(raw: &str) -> Result<Self, String> {
        if raw.is_empty() || raw.contains('*') || raw.chars().any(char::is_whitespace) {
            return Err(format!("invalid network origin: {raw:?}"));
        }
        let parsed = Url::parse(raw).map_err(|e| format!("invalid network origin {raw:?}: {e}"))?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(format!("network origin must not contain credentials: {raw:?}"));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(format!("network origin must not contain query or fragment: {raw:?}"));
        }
        if !matches!(parsed.path(), "" | "/") || raw.ends_with('/') {
            return Err(format!("network origin must not contain a path: {raw:?}"));
        }
        Self::from_url(&parsed)
    }

    /// Extracts the normalized origin from an already-parsed URL.
    pub fn from_url(url: &Url) -> Result<Self, String> {
        let scheme = OriginScheme::parse(url.scheme())
            .ok_or_else(|| format!("unsupported URL scheme: {:?}", url.scheme()))?;
        let host = match url.host() {
            Some(Host::Domain(domain)) => {
                if domain.is_empty() || domain.contains('*') {
                    return Err(format!("invalid host in URL: {url}"));
                }
                domain.to_ascii_lowercase()
            }
            Some(Host::Ipv4(v4)) => v4.to_string(),
            Some(Host::Ipv6(v6)) => format!("[{v6}]"),
            None => return Err(format!("URL is missing host: {url}")),
        };
        let port = url.port().unwrap_or_else(|| scheme.default_port());
        if port == 0 {
            return Err(format!("invalid port 0 in URL: {url}"));
        }
        Ok(Self { scheme, host, port })
    }

    /// Normalized scheme.
    #[must_use]
    pub const fn scheme(&self) -> OriginScheme {
        self.scheme
    }

    /// Normalized lowercase host (IPv6 bracketed).
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Effective port (default port when omitted in input).
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Canonical string form (`scheme://host` for default port, else `scheme://host:port`).
    #[must_use]
    pub fn canonical(&self) -> String {
        if self.port == self.scheme.default_port() {
            format!("{}://{}", self.scheme.as_str(), self.host)
        } else {
            format!("{}://{}:{}", self.scheme.as_str(), self.host, self.port)
        }
    }

    /// Host string without IPv6 brackets, suitable for DNS lookup and TLS SNI.
    #[must_use]
    pub fn connect_host(&self) -> &str {
        self.host
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .unwrap_or(&self.host)
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

/// Sealed per-session origin and destination network policy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OriginPolicy {
    origins: BTreeSet<Origin>,
    allow_insecure_http: bool,
    allow_private_networks: bool,
}

impl OriginPolicy {
    /// Builds a validated policy from manifest declarations.
    pub fn new(
        origins: impl IntoIterator<Item = Origin>,
        allow_insecure_http: bool,
        allow_private_networks: bool,
    ) -> Result<Self, String> {
        let mut set = BTreeSet::new();
        for origin in origins {
            if !origin.scheme().is_tls() && !allow_insecure_http {
                return Err(format!(
                    "origin {origin} requires capabilities.network.allowInsecureHttp = true"
                ));
            }
            if !allow_private_networks && is_restricted_host(origin.connect_host()) {
                return Err(format!(
                    "private/loopback origin {origin} requires capabilities.network.allowPrivateNetworks = true"
                ));
            }
            set.insert(origin);
        }
        Ok(Self {
            origins: set,
            allow_insecure_http,
            allow_private_networks,
        })
    }

    /// Whether this policy grants any network origin.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    /// Whether plaintext `http://` / `ws://` origins are permitted.
    #[must_use]
    pub const fn allow_insecure_http(&self) -> bool {
        self.allow_insecure_http
    }

    /// Whether loopback/private/link-local destinations are permitted.
    #[must_use]
    pub const fn allow_private_networks(&self) -> bool {
        self.allow_private_networks
    }

    /// Authorizes an HTTP/HTTPS URL before DNS lookup.
    pub fn authorize_http(&self, raw_url: &str) -> Result<(Url, Origin), NetworkError> {
        let (url, origin) = self.parse_and_authorize(raw_url)?;
        if !origin.scheme().is_http() {
            return Err(NetworkError::new(
                "E_NETWORK_POLICY",
                format!("HTTP request requires http/https scheme, got {}", origin.scheme().as_str()),
            ));
        }
        Ok((url, origin))
    }

    /// Authorizes a WS/WSS URL before DNS lookup.
    pub fn authorize_ws(&self, raw_url: &str) -> Result<(Url, Origin), NetworkError> {
        let (url, origin) = self.parse_and_authorize(raw_url)?;
        if !origin.scheme().is_ws() {
            return Err(NetworkError::new(
                "E_NETWORK_POLICY",
                format!("WebSocket requires ws/wss scheme, got {}", origin.scheme().as_str()),
            ));
        }
        Ok((url, origin))
    }

    fn parse_and_authorize(&self, raw_url: &str) -> Result<(Url, Origin), NetworkError> {
        let parsed = Url::parse(raw_url)
            .map_err(|e| NetworkError::new("E_NETWORK_POLICY", format!("invalid URL: {e}")))?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(NetworkError::new(
                "E_NETWORK_POLICY",
                "URLs with embedded credentials are forbidden",
            ));
        }
        let origin = Origin::from_url(&parsed)
            .map_err(|msg| NetworkError::new("E_NETWORK_POLICY", msg))?;
        if !origin.scheme().is_tls() && !self.allow_insecure_http {
            return Err(NetworkError::new(
                "E_CAPABILITY",
                format!("insecure scheme denied for origin {origin}"),
            ));
        }
        if !self.origins.contains(&origin) {
            return Err(NetworkError::new(
                "E_CAPABILITY",
                format!("origin {origin} is not in capabilities.network.origins"),
            ));
        }
        if !self.allow_private_networks && is_restricted_host(origin.connect_host()) {
            return Err(NetworkError::new(
                "E_NETWORK_POLICY",
                format!("private/loopback host denied for origin {origin}"),
            ));
        }
        Ok((parsed, origin))
    }

    /// Verifies a resolved IP address right before connecting.
    pub fn verify_destination_ip(&self, ip: IpAddr) -> Result<(), NetworkError> {
        if !self.allow_private_networks && is_restricted_ip(ip) {
            return Err(NetworkError::new(
                "E_NETWORK_POLICY",
                format!("connection to restricted IP {ip} denied by network policy"),
            ));
        }
        Ok(())
    }
}

/// Returns true when a literal hostname refers to localhost or a restricted IP literal.
#[must_use]
pub fn is_restricted_host(host: &str) -> bool {
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return true;
    }
    if let Ok(ip) = lower.parse::<IpAddr>() {
        return is_restricted_ip(ip);
    }
    false
}

/// Returns true for loopback, private, link-local, unspecified, multicast, CGNAT, and broadcast IPs.
#[must_use]
pub fn is_restricted_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_restricted_v4(v4),
        IpAddr::V6(v6) => is_restricted_v6(v6),
    }
}

fn is_restricted_v4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || (a == 100 && (64..=127).contains(&b)) // RFC 6598 CGNAT 100.64.0.0/10
        || a == 0
}

fn is_restricted_v6(ip: Ipv6Addr) -> bool {
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_restricted_v4(mapped);
    }
    let seg0 = ip.segments()[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (seg0 & 0xfe00) == 0xfc00 // ULA fc00::/7
        || (seg0 & 0xffc0) == 0xfe80 // Link-local fe80::/10
}

/// Verified transport stream (plain TCP or rustls TLS) connected strictly after IP policy verification.
pub enum VerifiedStream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for VerifiedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Plain(s) => Pin::new(s).poll_read(cx, buf),
            Self::Tls(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for VerifiedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match &mut *self {
            Self::Plain(s) => Pin::new(s).poll_write(cx, buf),
            Self::Tls(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Plain(s) => Pin::new(s).poll_flush(cx),
            Self::Tls(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &mut *self {
            Self::Plain(s) => Pin::new(s).poll_shutdown(cx),
            Self::Tls(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

fn tls_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let mut config = ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .expect("valid default rustls protocol versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            Arc::new(config)
        })
        .clone()
}

/// Resolves DNS, verifies every candidate IP against `policy`, connects TCP, and upgrades TLS when required.
pub async fn connect_verified_stream(
    origin: &Origin,
    policy: &OriginPolicy,
    connect_timeout: Duration,
) -> Result<VerifiedStream, NetworkError> {
    let host = origin.connect_host();
    let port = origin.port();
    let addrs: Vec<SocketAddr> = timeout(connect_timeout, lookup_host((host, port)))
        .await
        .map_err(|_| NetworkError::new("E_TIMEOUT", format!("DNS lookup timed out for {origin}")))?
        .map_err(|e| NetworkError::new("E_NETWORK", format!("DNS lookup failed for {origin}: {e}")))?
        .collect();

    if addrs.is_empty() {
        return Err(NetworkError::new("E_NETWORK", format!("DNS returned no addresses for {origin}")));
    }

    for addr in &addrs {
        policy.verify_destination_ip(addr.ip())?;
    }

    let mut last_err = None;
    let mut connected = None;
    for addr in addrs {
        match timeout(connect_timeout, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                connected = Some(stream);
                break;
            }
            Ok(Err(e)) => last_err = Some(e.to_string()),
            Err(_) => last_err = Some("TCP connect timed out".into()),
        }
    }
    let tcp = connected.ok_or_else(|| {
        NetworkError::new(
            "E_NETWORK",
            format!("failed to connect to {origin}: {}", last_err.unwrap_or_default()),
        )
    })?;

    if !origin.scheme().is_tls() {
        return Ok(VerifiedStream::Plain(tcp));
    }

    let server_name = ServerName::try_from(host.to_owned()).map_err(|_| {
        NetworkError::new("E_NETWORK_POLICY", format!("invalid TLS server name for {origin}"))
    })?;
    let connector = TlsConnector::from(tls_config());
    let tls = timeout(connect_timeout, connector.connect(server_name, tcp))
        .await
        .map_err(|_| NetworkError::new("E_TIMEOUT", format!("TLS handshake timed out for {origin}")))?
        .map_err(|e| NetworkError::new("E_NETWORK", format!("TLS handshake failed for {origin}: {e}")))?;
    Ok(VerifiedStream::Tls(Box::new(tls)))
}
