//! Explicit scans for local DNS-SD advertisements and authorized Android devices.
use crate::{
    filesystem::Entry,
    remote::{Connection, FtpTls, Secrets},
};
use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::{
    collections::BTreeMap,
    net::IpAddr,
    path::PathBuf,
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
    browse(|_| {})
}

fn browse(publish: impl FnMut(&BTreeMap<String, Connection>)) -> Result<Vec<Connection>> {
    let daemon = ServiceDaemon::new()?;
    let result = scan(&daemon, publish);
    let _ = daemon.shutdown();
    result
}

fn scan(
    daemon: &ServiceDaemon,
    mut publish: impl FnMut(&BTreeMap<String, Connection>),
) -> Result<Vec<Connection>> {
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
        let mut changed = false;
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
                            changed |= connections
                                .get(&key)
                                .is_none_or(|c: &Connection| c.endpoint != endpoint);
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
                        changed |= connections.remove(&name.to_ascii_lowercase()).is_some();
                    }
                    _ => {}
                }
            }
        }
        if changed {
            publish(&connections);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    Ok(connections.into_values().collect())
}

pub(crate) fn network_entries(mut publish: impl FnMut(Vec<Entry>)) -> Result<Vec<Entry>> {
    let mounted: Vec<_> = crate::native_files::network_mounts()
        .into_iter()
        .map(|(name, path)| folder(name, path))
        .collect();
    if !mounted.is_empty() {
        publish(mounted.clone());
    }

    fn entries<'a>(
        mounted: &[Entry],
        connections: impl IntoIterator<Item = &'a Connection>,
    ) -> Vec<Entry> {
        let mut seen = std::collections::HashSet::new();
        let mut rows = mounted.to_vec();
        rows.extend(
            connections
                .into_iter()
                .filter(|c| seen.insert(&c.endpoint))
                .map(|c| {
                    folder(
                        format!("{} ({})", c.name, c.service.to_ascii_uppercase()),
                        &c.endpoint,
                    )
                }),
        );
        rows
    }
    match browse(|connections| publish(entries(&mounted, connections.values()))) {
        Ok(found) => Ok(entries(&mounted, &found)),
        Err(_) if !mounted.is_empty() => {
            crate::debug_log::record("warning", "Bonjour discovery failed while listing Network.");
            Ok(mounted)
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn server_entries(profile: &Connection, secrets: &Secrets) -> Result<Vec<Entry>> {
    let names = match profile.service.as_str() {
        "smb" => crate::smb::list_shares(profile, secrets)?,
        "nfs" => crate::uri_backend::nfs::list_exports(&profile.endpoint)?,
        _ => anyhow::bail!("This service does not have a server listing."),
    };
    Ok(names
        .into_iter()
        .map(|name| folder(name.clone(), name))
        .collect())
}

fn folder(name: String, path: impl Into<PathBuf>) -> Entry {
    Entry {
        path: path.into(),
        name,
        is_dir: true,
        is_symlink: false,
        size: 0,
        size_known: false,
        modified: None,
    }
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
        let result = scan(&browser, |_| {});
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
