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
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
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

/// The instance name shown on the phone. Not a security boundary, so a fallback is fine.
pub fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "conduit-desktop".to_string())
}
