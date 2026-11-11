//! mDNS advertising.
//!
//! The phone browses for `_conduit._tcp` in short bursts, so this side has to be the
//! one that is always answerable. `enable_addr_auto` is why: the daemon tracks interface
//! addresses itself, so a docking station or a VPN coming up does not require a
//! re-register — and therefore does not disturb the live session at all.
//!
//! Instance names are not identities. The TXT record carries the device id and
//! fingerprint so the phone can tell two desktops apart, and pairing still verifies the
//! Noise static key; a spoofed advert only wastes one handshake.

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use tracing::info;

const SERVICE_TYPE: &str = "_conduit._tcp.local.";

/// Keeps the registration alive. Dropping it deregisters and stops the daemon thread.
pub struct Advert(ServiceDaemon);

impl Advert {
    pub fn start(port: u16, device_id: &str, fingerprint: &str) -> Result<Self> {
        let host = hostname();
        let properties = HashMap::from([
            ("id".to_string(), device_id.to_string()),
            ("fp".to_string(), fingerprint.to_string()),
            ("v".to_string(), "1".to_string()),
        ]);

        // Empty address list plus addr_auto: the daemon fills in and maintains them.
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &host,
            &format!("{host}.local."),
            (),
            port,
            properties,
        )
        .context("building the mDNS service record")?
        .enable_addr_auto();

        let daemon = ServiceDaemon::new().context("starting the mDNS daemon")?;
        daemon.register(service).context("registering over mDNS")?;
        info!(%host, port, "advertising {SERVICE_TYPE}");
        Ok(Self(daemon))
    }
}

impl Drop for Advert {
    fn drop(&mut self) {
        // Best effort: a goodbye packet is polite, but the phone's own read deadline is
        // what it actually relies on.
        let _ = self.0.shutdown();
        info!("mDNS advert withdrawn");
    }
}

/// Finds a paired peer's LAN socket by its device id.
///
/// One bounded browse, started and torn down per call: the mDNS daemon thread is shut down as
/// soon as the peer is found or the deadline passes, so an idle desktop runs no discovery at all.
/// A stale or spoofed advert only yields an address whose Noise handshake then fails.
pub async fn discover_lan_peer(target_id: &str, timeout: Duration) -> Option<SocketAddr> {
    let daemon = ServiceDaemon::new().ok()?;
    let receiver = daemon.browse(SERVICE_TYPE).ok()?;
    let target = target_id.to_owned();
    let found = tokio::task::spawn_blocking(move || {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match receiver.recv_timeout(remaining) {
                Ok(ServiceEvent::ServiceResolved(info)) => {
                    if info.get_property_val_str("id") != Some(target.as_str()) {
                        continue;
                    }
                    let address = info.get_addresses().iter().next()?.to_ip_addr();
                    return Some(SocketAddr::new(address, info.get_port()));
                }
                Ok(_) => {}
                Err(_) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten();
    let _ = daemon.shutdown();
    found
}

/// The Windows device name shown on the phone. Not a security boundary.
///
/// Do not rely on `COMPUTERNAME`: service/AgentDock/WMI launch paths can omit that environment
/// variable, which previously made a real machine advertise itself as `conduit-desktop`.
/// `conduit-desktop`. ActiveComputerName is the same system identity Windows exposes in Settings.
pub fn hostname() -> String {
    // TCP/IP's Hostname preserves the casing chosen in Windows Settings (for example `Log`).
    // ActiveComputerName is the NetBIOS form and Windows commonly normalises it to `LOG`.
    windows_registry::LOCAL_MACHINE
        .open(r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters")
        .ok()
        .and_then(|key| key.get_string("Hostname").ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .or_else(|| {
            windows_registry::LOCAL_MACHINE
                .open(r"SYSTEM\CurrentControlSet\Control\ComputerName\ActiveComputerName")
                .ok()
                .and_then(|key| key.get_string("ComputerName").ok())
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
        })
        .or_else(|| {
            std::env::var("COMPUTERNAME")
                .ok()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
        })
        .unwrap_or_else(|| "conduit-desktop".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn discovery_is_bounded_and_returns_nothing_for_an_absent_peer() {
        let started = Instant::now();
        let found = discover_lan_peer(&"Z".repeat(43), Duration::from_millis(150)).await;
        assert!(found.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "a failed discovery must stay bounded, took {:?}",
            started.elapsed()
        );
    }
}
