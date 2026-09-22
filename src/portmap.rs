//! Router port mapping via UPnP IGD — the "Port Mapping (UPnP)" half of the
//! Remote Access feature (reach.rs). Asks the home router to forward
//! `http_port` to this machine so a Clay client on the internet can connect.
//!
//! Lifecycle (driven by `App::start_port_mapping` & friends in main.rs):
//! map after the HTTPS server starts, re-add every [`RENEW_EVERY`] (a UPnP
//! "renew" is just `AddPortMapping` again with the same values), remove on
//! toggle-off / port change / clean quit — but *not* on a hot reload, since the
//! new process re-adds the same mapping. A crash leaves the mapping until its
//! [`LEASE_SECS`] lease expires; routers that only do permanent leases keep it
//! until the next Clay start removes/re-adds it.
//!
//! What can go wrong, and how it is reported (`reach::PortMapStatus`):
//! * no gateway answers the SSDP search → `NoGateway` (UPnP disabled on the
//!   router, or no router) — forward the port by hand;
//! * the router's own WAN address is private (RFC 1918 / 100.64/10 CGNAT) →
//!   `DoubleNat`: the mapping was made, but a second NAT sits between the router
//!   and the internet, so nothing outside can reach it — use `--ssh` or a VPN;
//! * the external port is taken by another device's mapping → fall back to
//!   `AddAnyPortMapping` and report the port the router chose;
//! * the router only accepts permanent leases → retry with lease 0.
//!
//! Plain HTTP SOAP to the router's LAN address, pure Rust (`igd-next`): no TLS
//! backend is involved, so the `ring`-only policy in Cargo.toml is unaffected.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use igd_next::aio::tokio::{search_gateway, Tokio};
use igd_next::aio::Gateway;
use igd_next::{AddAnyPortError, AddPortError, PortMappingProtocol, RemovePortError, SearchOptions};

use crate::reach::{self, PortMapStatus};

/// Lease requested from the router, in seconds. One hour: long enough that a
/// missed renewal doesn't drop the mapping, short enough that a crash doesn't
/// leave a stale forward around for long.
pub const LEASE_SECS: u32 = 3600;
/// How often the mapping is re-added while enabled (well inside the lease).
pub const RENEW_EVERY: Duration = Duration::from_secs(20 * 60);
/// Shown in the router's port-forwarding table.
pub const DESCRIPTION: &str = "Clay MUD client";

const SEARCH_TIMEOUT: Duration = Duration::from_secs(3);
const MAP_TIMEOUT: Duration = Duration::from_secs(15);
const UNMAP_TIMEOUT: Duration = Duration::from_secs(8);

/// External port of the mapping this process currently holds, if any. Kept
/// outside `App` so the GUI window-close path (a plain tao thread with no access
/// to the app state) can still remove it — see [`unmap_active_blocking`].
static ACTIVE_EXTERNAL_PORT: std::sync::Mutex<Option<u16>> = std::sync::Mutex::new(None);

/// Record (or clear) the mapping this process holds.
pub fn set_active(external_port: Option<u16>) {
    if let Ok(mut g) = ACTIVE_EXTERNAL_PORT.lock() {
        *g = external_port;
    }
}

/// The external port currently held, if any.
pub fn active() -> Option<u16> {
    ACTIVE_EXTERNAL_PORT.lock().ok().and_then(|g| *g)
}

/// Clear and return the held external port (so two exit paths never both try
/// to remove it).
pub fn take_active() -> Option<u16> {
    ACTIVE_EXTERNAL_PORT.lock().ok().and_then(|mut g| g.take())
}

/// Best-effort removal of the held mapping from a thread that is *not* inside
/// the tokio runtime (the GUI window-close path). Builds a throwaway runtime and
/// waits at most the unmap timeout; a hot reload must NOT call this (the new
/// process re-adds the same mapping).
pub fn unmap_active_blocking() {
    let Some(port) = take_active() else { return };
    if let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() {
        let _ = rt.block_on(unmap_port(port));
    }
}

fn search_options() -> SearchOptions {
    SearchOptions { timeout: Some(SEARCH_TIMEOUT), ..Default::default() }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Ask the router to forward TCP `port` to this machine. Never panics and never
/// hangs: every outcome is a `PortMapStatus`, and the whole exchange is capped
/// at [`MAP_TIMEOUT`].
pub async fn map_port(port: u16) -> PortMapStatus {
    match tokio::time::timeout(MAP_TIMEOUT, map_port_inner(port)).await {
        Ok(status) => status,
        Err(_) => PortMapStatus::Failed { error: "the router did not answer in time".to_string() },
    }
}

async fn map_port_inner(port: u16) -> PortMapStatus {
    let gateway = match search_gateway(search_options()).await {
        Ok(g) => g,
        Err(_) => return PortMapStatus::NoGateway,
    };
    let gw_ip = match gateway.addr.ip() {
        IpAddr::V4(v4) => v4,
        IpAddr::V6(_) => return PortMapStatus::Failed { error: "IPv6-only gateway is not supported".to_string() },
    };
    // The address the router must forward to is the one on the interface that
    // faces it — not whatever interface reaches 8.8.8.8 on a multi-homed host.
    let Some(lan_ip) = reach::lan_ip_toward(gw_ip) else {
        return PortMapStatus::Failed { error: format!("no route to gateway {gw_ip}") };
    };
    let external_ip = match gateway.get_external_ip().await {
        Ok(ip) => ip,
        Err(e) => return PortMapStatus::Failed { error: format!("GetExternalIPAddress: {e}") },
    };
    let local = SocketAddr::V4(SocketAddrV4::new(lan_ip, port));
    match add_with_fallbacks(&gateway, port, local).await {
        Ok((external_port, lease_secs)) => status_for(external_ip, external_port, gw_ip, lan_ip, lease_secs, unix_now()),
        Err(error) => PortMapStatus::Failed { error },
    }
}

/// `AddPortMapping` with the two fallbacks real routers need: lease 0 for the
/// ones that only do permanent mappings, and `AddAnyPortMapping` when the
/// external port is already taken by another device (or the router insists the
/// external and internal ports match and ours doesn't fit). Returns the external
/// port actually mapped and the lease it was granted with.
async fn add_with_fallbacks(gw: &Gateway<Tokio>, port: u16, local: SocketAddr) -> Result<(u16, u32), String> {
    match gw.add_port(PortMappingProtocol::TCP, port, local, LEASE_SECS, DESCRIPTION).await {
        Ok(()) => return Ok((port, LEASE_SECS)),
        Err(AddPortError::OnlyPermanentLeasesSupported) => {
            return gw
                .add_port(PortMappingProtocol::TCP, port, local, 0, DESCRIPTION)
                .await
                .map(|_| (port, 0))
                .map_err(|e| format!("AddPortMapping (permanent lease): {e}"));
        }
        Err(AddPortError::PortInUse) | Err(AddPortError::SamePortValuesRequired) => {}
        Err(e) => return Err(format!("AddPortMapping: {e}")),
    }
    match gw.add_any_port(PortMappingProtocol::TCP, local, LEASE_SECS, DESCRIPTION).await {
        Ok(p) => Ok((p, LEASE_SECS)),
        Err(AddAnyPortError::OnlyPermanentLeasesSupported) => gw
            .add_any_port(PortMappingProtocol::TCP, local, 0, DESCRIPTION)
            .await
            .map(|p| (p, 0))
            .map_err(|e| format!("AddAnyPortMapping (permanent lease): {e}")),
        Err(e) => Err(format!("AddAnyPortMapping: {e}")),
    }
}

/// Classify a successful mapping: a private WAN address means the router is
/// itself behind another NAT, so the mapping exists but cannot be reached from
/// the internet (`DoubleNat`); otherwise it is `Mapped`.
pub fn status_for(
    external_ip: IpAddr,
    external_port: u16,
    gateway: Ipv4Addr,
    lan_ip: Ipv4Addr,
    lease_secs: u32,
    since_secs: u64,
) -> PortMapStatus {
    match external_ip {
        IpAddr::V4(v4) if reach::is_private_v4(v4) => {
            PortMapStatus::DoubleNat { external_ip: v4.to_string(), external_port }
        }
        ip => PortMapStatus::Mapped {
            external_ip: ip.to_string(),
            external_port,
            gateway: gateway.to_string(),
            lan_ip: lan_ip.to_string(),
            lease_secs,
            since_secs,
        },
    }
}

/// Remove the mapping for `external_port`. "No such mapping" counts as success
/// (the lease may already have expired). Capped at [`UNMAP_TIMEOUT`].
pub async fn unmap_port(external_port: u16) -> Result<(), String> {
    let work = async {
        let gateway = search_gateway(search_options()).await.map_err(|e| format!("no UPnP gateway: {e}"))?;
        match gateway.remove_port(PortMappingProtocol::TCP, external_port).await {
            Ok(()) | Err(RemovePortError::NoSuchPortMapping) => Ok(()),
            Err(e) => Err(format!("DeletePortMapping: {e}")),
        }
    };
    match tokio::time::timeout(UNMAP_TIMEOUT, work).await {
        Ok(r) => r,
        Err(_) => Err("the router did not answer in time".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn public_wan_address_is_mapped() {
        let s = status_for("203.0.113.5".parse().unwrap(), 9000, v4("192.168.1.1"), v4("192.168.1.20"), LEASE_SECS, 42);
        assert_eq!(
            s,
            PortMapStatus::Mapped {
                external_ip: "203.0.113.5".into(),
                external_port: 9000,
                gateway: "192.168.1.1".into(),
                lan_ip: "192.168.1.20".into(),
                lease_secs: LEASE_SECS,
                since_secs: 42,
            }
        );
    }

    #[test]
    fn private_or_cgnat_wan_address_is_double_nat() {
        for wan in ["192.168.0.2", "10.0.0.7", "172.20.1.1", "100.70.1.2"] {
            let s = status_for(wan.parse().unwrap(), 9001, v4("192.168.1.1"), v4("192.168.1.20"), 0, 0);
            assert_eq!(s, PortMapStatus::DoubleNat { external_ip: wan.into(), external_port: 9001 }, "{wan}");
        }
    }

    #[test]
    fn renew_interval_is_inside_the_lease() {
        assert!(RENEW_EVERY.as_secs() * 2 < LEASE_SECS as u64);
    }
}
