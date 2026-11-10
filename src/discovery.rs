//! Explicit scans for local DNS-SD advertisements and authorized Android devices.
use crate::remote::{Connection, FtpTls};
use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::{
    collections::BTreeMap,
    net::IpAddr,
    time::{Duration, Instant},
};

pub(crate) fn android() -> Result<Vec<Connection>> {
    Ok(crate::uri_backend::adb::devices()?
        .into_iter()
        .map(|(serial, name)| Connection {
            name,
            service: "adb".into(),
            endpoint: "adb://localhost".into(),
            bucket: serial,
            listed: false,
            ..Default::default()
        })
        .collect())
}

pub(crate) fn bonjour() -> Result<Vec<Connection>> {
    let daemon = ServiceDaemon::new()?;
    let result = scan(&daemon);
    let _ = daemon.shutdown();
    result
}

fn scan(daemon: &ServiceDaemon) -> Result<Vec<Connection>> {
    let types = [
        ("_smb._tcp.local.", "smb", "smb"),
        ("_sftp-ssh._tcp.local.", "sftp", "ssh"),
        ("_ftp._tcp.local.", "ftp", "ftp"),
        ("_nfs._tcp.local.", "nfs", "nfs"),
    ];
    let mut browsers = Vec::new();
    for (ty, service, scheme) in types {
        browsers.push((ty, service, scheme, daemon.browse(ty)?));
    }
    let mut connections = BTreeMap::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        for (ty, service, scheme, browser) in &browsers {
            for event in browser.try_iter().take(256) {
                match event {
                    ServiceEvent::ServiceResolved(resolved) if resolved.get_port() != 0 => {
                        let address = resolved
                            .get_addresses()
                            .iter()
                            .map(|ip| ip.to_ip_addr())
                            .filter(|ip| match ip {
                                IpAddr::V4(ip) => !ip.is_unspecified() && !ip.is_multicast(),
                                IpAddr::V6(ip) => {
                                    !ip.is_unspecified()
                                        && !ip.is_multicast()
                                        && !ip.is_unicast_link_local()
                                }
                            })
                            .min();
                        if let Some(address) = address {
                            let fullname = resolved.get_fullname();
                            let key = fullname.to_ascii_lowercase();
                            if connections.len() >= 256 && !connections.contains_key(&key) {
                                continue;
                            }
                            let endpoint = format!(
                                "{scheme}://{}",
                                std::net::SocketAddr::new(address, resolved.get_port())
                            );
                            let name = fullname
                                .strip_suffix(ty)
                                .unwrap_or(fullname)
                                .trim_end_matches('.');
                            connections.insert(
                                key,
                                Connection {
                                    name: name.into(),
                                    service: (*service).into(),
                                    endpoint,
                                    ftp_tls: if *service == "ftp" {
                                        FtpTls::Plain
                                    } else {
                                        FtpTls::Automatic
                                    },
                                    listed: false,
                                    ..Default::default()
                                },
                            );
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, name) => {
                        connections.remove(&name.to_ascii_lowercase());
                    }
                    _ => {}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    Ok(connections.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdns_sd::{IfKind, ServiceInfo};
    use std::net::UdpSocket;

    #[test]
    #[ignore = "requires loopback multicast"]
    fn bonjour_resolves_a_loopback_advertisement() {
        let port = UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let fixture = ServiceDaemon::new_with_port(port).unwrap();
        let browser = ServiceDaemon::new_with_port(port).unwrap();
        for daemon in [&fixture, &browser] {
            daemon.disable_interface(IfKind::All).unwrap();
            daemon.enable_interface(IfKind::LoopbackV4).unwrap();
        }
        fixture
            .register(
                ServiceInfo::new(
                    "_smb._tcp.local.",
                    "Rust Commander fixture",
                    "rust-commander-fixture.local.",
                    "127.0.0.1",
                    445,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        let result = scan(&browser);
        fixture.shutdown().unwrap();
        browser.shutdown().unwrap();
        assert!(result.unwrap().iter().any(|c| {
            c.name == "Rust Commander fixture"
                && c.service == "smb"
                && c.endpoint == "smb://127.0.0.1:445"
                && !c.listed
        }));
    }
}
