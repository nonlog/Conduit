//! Shared iroh QUIC WAN transport for Conduit (Windows daemon + Android JNI).
//!
//! Bridges authenticated iroh QUIC bidirectional streams onto local loopback TCP
//! sockets so Conduit's existing Noise XX + Protobuf wire protocol runs unchanged
//! without requiring users to re-pair devices.

use anyhow::{bail, Context, Result};
use iroh::{
    address_lookup::{AddressLookupBuilder, DnsAddressLookup, PkarrPublisher, PkarrResolver},
    defaults::prod,
    endpoint::{presets, Connection, QuicTransportConfig, RecvStream, SendStream},
    Endpoint, EndpointAddr, PublicKey, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey,
};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};
use url::Url;

pub const ALPN: &[u8] = b"conduit/iroh/1";
const KEY_DERIVE_DOMAIN: &[u8] = b"conduit-iroh-ed25519-v1:";
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
const IDLE_TIMEOUT: Duration = Duration::from_secs(45);

/// Deterministically derives an Ed25519 iroh SecretKey from Conduit's 32-byte
/// Noise X25519 private key so no second key file is needed on disk.
pub fn derive_secret_key(noise_private_key: &[u8]) -> Result<SecretKey> {
    if noise_private_key.len() != 32 {
        bail!(
            "expected 32-byte Noise private key, got {}",
            noise_private_key.len()
        );
    }
    let mut hasher = Sha256::new();
    hasher.update(KEY_DERIVE_DOMAIN);
    hasher.update(noise_private_key);
    let seed: [u8; 32] = hasher.finalize().into();
    Ok(SecretKey::from_bytes(&seed))
}

/// Formats an iroh EndpointId (32-byte Ed25519 public key) as 64 lowercase hex chars.
pub fn endpoint_id_hex(public: &PublicKey) -> String {
    let mut out = String::with_capacity(64);
    for b in public.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Parses a 64-character hex string back into an iroh PublicKey (EndpointId).
pub fn parse_endpoint_id(hex: &str) -> Result<PublicKey> {
    let hex = hex.trim();
    if hex.len() != 64 {
        bail!("iroh endpoint id must be 64 hex chars, got {}", hex.len());
    }
    let mut bytes = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
        let s = std::str::from_utf8(chunk).context("non-UTF8 hex")?;
        bytes[i] = u8::from_str_radix(s, 16).context("invalid hex byte")?;
    }
    PublicKey::from_bytes(&bytes).map_err(|e| anyhow::anyhow!("invalid iroh public key: {e}"))
}

pub fn parse_relay_url(raw: &str) -> Result<RelayUrl> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 1024 {
        bail!("empty or oversized relay URL");
    }
    let candidate = if trimmed.contains("://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    };
    let url = Url::parse(&candidate).context("invalid relay URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        bail!("relay URL must use http or https with a host");
    }
    Ok(url.into())
}

pub fn build_relay_map(custom_relays: &[String], use_default_relays: bool) -> RelayMap {
    let mut configs: Vec<RelayConfig> = if use_default_relays {
        prod::default_relay_map().relays::<Vec<_>>()
    } else {
        Vec::new()
    };
    for raw in custom_relays {
        match parse_relay_url(raw) {
            Ok(url) => {
                if !configs.iter().any(|c| c.url == url) {
                    configs.push(RelayConfig::from(url));
                }
            }
            Err(err) => warn!(relay = %raw, %err, "ignoring invalid custom iroh relay URL"),
        }
    }
    configs.into_iter().collect()
}

fn route_label(conn: &Connection) -> String {
    let direct = conn.paths().iter().any(|p| !p.is_relay());
    if direct {
        "iroh-direct".to_owned()
    } else {
        "iroh-relay".to_owned()
    }
}

/// Combines iroh's separate SendStream and RecvStream into one AsyncRead + AsyncWrite
/// handle for `tokio::io::copy_bidirectional`.
struct BiStream {
    send: SendStream,
    recv: RecvStream,
}

impl AsyncRead for BiStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

impl AsyncWrite for BiStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.send)
            .poll_write(cx, buf)
            .map_err(std::io::Error::other)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_shutdown(cx)
    }
}

pub struct IrohNode {
    endpoint: Endpoint,
    endpoint_id: String,
}

impl IrohNode {
    pub async fn start(
        noise_private_key: &[u8],
        custom_relays: &[String],
        use_default_relays: bool,
        proxy: Option<&str>,
    ) -> Result<Self> {
        let secret = derive_secret_key(noise_private_key)?;
        let endpoint_id = endpoint_id_hex(&secret.public());
        let relay_map = build_relay_map(custom_relays, use_default_relays);

        let transport = QuicTransportConfig::builder()
            .keep_alive_interval(KEEP_ALIVE_INTERVAL)
            .max_idle_timeout(Some(
                IDLE_TIMEOUT
                    .try_into()
                    .context("idle timeout exceeds QUIC varint")?,
            ))
            .build();

        let mut builder = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .alpns(vec![ALPN.to_vec()])
            .relay_mode(RelayMode::Custom(relay_map))
            .transport_config(transport);

        if let Some(raw_proxy) = proxy.map(str::trim).filter(|s| !s.is_empty()) {
            if let Ok(url) = Url::parse(raw_proxy) {
                builder = builder.proxy_url(url);
            } else {
                warn!(proxy = %raw_proxy, "ignoring unparseable proxy URL for iroh");
            }
        }

        let endpoint = builder
            .bind()
            .await
            .map_err(|e| anyhow::anyhow!("binding iroh endpoint: {e}"))?;

        let lookups = endpoint
            .address_lookup()
            .map_err(|e| anyhow::anyhow!("accessing iroh address lookup: {e}"))?;
        lookups.add(
            PkarrPublisher::n0_dns()
                .into_address_lookup(&endpoint)
                .map_err(|e| anyhow::anyhow!("adding PkarrPublisher: {e}"))?,
        );
        lookups.add(
            PkarrResolver::n0_dns()
                .into_address_lookup(&endpoint)
                .map_err(|e| anyhow::anyhow!("adding PkarrResolver: {e}"))?,
        );
        lookups.add(
            DnsAddressLookup::n0_dns()
                .into_address_lookup(&endpoint)
                .map_err(|e| anyhow::anyhow!("adding DnsAddressLookup: {e}"))?,
        );

        info!(
            endpoint_id = %&endpoint_id[..12],
            default_relays = use_default_relays,
            custom_relays = custom_relays.len(),
            "iroh WAN endpoint started"
        );
        Ok(Self {
            endpoint,
            endpoint_id,
        })
    }

    pub fn endpoint_id(&self) -> &str {
        &self.endpoint_id
    }

    /// Accepts the next incoming iroh QUIC bi-stream and returns a bridged loopback
    /// `TcpStream` together with its route label (`iroh-direct` or `iroh-relay`).
    pub async fn accept_bridged(&self) -> Result<(TcpStream, String)> {
        loop {
            let Some(incoming) = self.endpoint.accept().await else {
                bail!("iroh endpoint closed");
            };
            let conn = match incoming.await {
                Ok(conn) => conn,
                Err(err) => {
                    debug!(%err, "iroh incoming connection handshake failed");
                    continue;
                }
            };
            let route = route_label(&conn);
            let (send, recv) = match conn.accept_bi().await {
                Ok(pair) => pair,
                Err(err) => {
                    debug!(%err, "iroh accept_bi failed");
                    continue;
                }
            };
            let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
            let local_addr = listener.local_addr()?;
            let client_fut = TcpStream::connect(local_addr);
            let accept_fut = listener.accept();
            let (client_res, server_res) = tokio::join!(client_fut, accept_fut);
            let client = client_res?;
            let (mut server, _) = server_res?;
            let _ = client.set_nodelay(true);
            let _ = server.set_nodelay(true);

            tokio::spawn(async move {
                let mut bi = BiStream { send, recv };
                let _ = tokio::io::copy_bidirectional(&mut server, &mut bi).await;
                conn.close(0u32.into(), b"done");
            });

            return Ok((client, route));
        }
    }

    /// Dials a peer by 64-char hex EndpointId and returns a bridged loopback `TcpStream`.
    pub async fn dial_bridged(
        &self,
        peer_hex: &str,
        timeout: Duration,
    ) -> Result<(TcpStream, String)> {
        let peer_key = parse_endpoint_id(peer_hex)?;
        let addr = EndpointAddr::new(peer_key);
        let conn = tokio::time::timeout(timeout, self.endpoint.connect(addr, ALPN))
            .await
            .context("iroh connect timed out")?
            .map_err(|e| anyhow::anyhow!("iroh connect failed: {e}"))?;
        let route = route_label(&conn);
        let (send, recv) = tokio::time::timeout(timeout, conn.open_bi())
            .await
            .context("iroh open_bi timed out")?
            .map_err(|e| anyhow::anyhow!("iroh open_bi failed: {e}"))?;

        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
        let local_addr = listener.local_addr()?;
        let (client_res, server_res) =
            tokio::join!(TcpStream::connect(local_addr), listener.accept());
        let client = client_res?;
        let (mut server, _) = server_res?;
        let _ = client.set_nodelay(true);
        let _ = server.set_nodelay(true);

        tokio::spawn(async move {
            let mut bi = BiStream { send, recv };
            let _ = tokio::io::copy_bidirectional(&mut server, &mut bi).await;
            conn.close(0u32.into(), b"done");
        });

        Ok((client, route))
    }

    /// Dials a peer by 64-char hex EndpointId, opens a single-accept loopback TCP
    /// bridge on `127.0.0.1:<port>`, and returns `(port, route_label)`.
    /// Used by Android's JNI bridge so Kotlin `Socket` connects seamlessly.
    pub async fn dial_local_port(
        &self,
        peer_hex: &str,
        timeout: Duration,
    ) -> Result<(u16, String)> {
        let peer_key = parse_endpoint_id(peer_hex)?;
        let addr = EndpointAddr::new(peer_key);
        let conn = tokio::time::timeout(timeout, self.endpoint.connect(addr, ALPN))
            .await
            .context("iroh connect timed out")?
            .map_err(|e| anyhow::anyhow!("iroh connect failed: {e}"))?;
        let route = route_label(&conn);
        let (send, recv) = tokio::time::timeout(timeout, conn.open_bi())
            .await
            .context("iroh open_bi timed out")?
            .map_err(|e| anyhow::anyhow!("iroh open_bi failed: {e}"))?;

        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
        let port = listener.local_addr()?.port();

        tokio::spawn(async move {
            let accepted = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await;
            let Ok(Ok((mut local, _))) = accepted else {
                conn.close(0u32.into(), b"local bridge timeout");
                return;
            };
            let _ = local.set_nodelay(true);
            let mut bi = BiStream { send, recv };
            let _ = tokio::io::copy_bidirectional(&mut local, &mut bi).await;
            conn.close(0u32.into(), b"done");
        });

        Ok((port, route))
    }

    pub async fn network_change(&self) {
        self.endpoint.network_change().await;
    }

    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_key_and_hex_roundtrip() {
        let priv_key = [42u8; 32];
        let k1 = derive_secret_key(&priv_key).unwrap();
        let k2 = derive_secret_key(&priv_key).unwrap();
        assert_eq!(k1.to_bytes(), k2.to_bytes());
        let hex = endpoint_id_hex(&k1.public());
        assert_eq!(hex.len(), 64);
        assert_eq!(parse_endpoint_id(&hex).unwrap(), k1.public());
    }

    #[test]
    fn relay_map_handles_defaults_and_custom() {
        let map = build_relay_map(&["relay.example.com".into()], true);
        assert!(map.len() >= 5);
        let custom_only = build_relay_map(&["https://relay.example.com".into()], false);
        assert_eq!(custom_only.len(), 1);
    }
}
