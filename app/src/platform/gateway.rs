//! Guessing a sensible first target: the default gateway of this machine.
//!
//! The app opens with three targets, and the first one is meant to be the local router — that is
//! the hop whose latency says most about a home network. Reading it out of the system's own routing
//! table beats shipping a fixed address, but the command differs per OS and its columns differ per
//! display language, so the parsing is deliberately loose: find the line for the default route and
//! take the first address on it that is neither the destination nor the netmask.

use std::process::{Command, Stdio};

/// A dotted-quad gateway, or `None` when nothing plausible was found — in which case the caller
/// falls back to the usual home-router address rather than leaving the field empty.
pub fn default_gateway() -> Option<String> {
    #[cfg(windows)]
    {
        // `route print 0.0.0.0` prints only the default route, which keeps the parsing below short.
        if let Some(text) = run_hidden("route", &["print", "0.0.0.0"]) {
            if let Some(g) = gateway_from_route_print(&text) {
                return Some(g);
            }
        }
        // Some Windows builds and language packs format that line differently; `ipconfig` is the
        // second chance because its "Default Gateway" label is easy to spot in any language.
        if let Some(text) = run_hidden("ipconfig", &[]) {
            return gateway_from_ipconfig(&text);
        }
    }

    #[cfg(target_os = "linux")]
    {
        // `ip route` is the modern command and its output is machine-stable: the line starting
        // `default` carries `via <address>`. `netstat -rn` is the fallback for a minimal system
        // without iproute2.
        if let Some(text) = run("ip", &["route", "show"]) {
            if let Some(g) = gateway_from_ip_route(&text) {
                return Some(g);
            }
        }
        if let Some(text) = run("netstat", &["-rn"]) {
            return gateway_from_netstat(&text);
        }
    }

    #[cfg(target_os = "macos")]
    {
        // `route -n get default` prints one route as `key: value` lines, including `gateway:`.
        if let Some(text) = run("route", &["-n", "get", "default"]) {
            if let Some(g) = gateway_from_colon_lines(&text) {
                return Some(g);
            }
        }
        // Older macOS releases, and a machine where `route` is not on PATH: `netstat -rn` works
        // there too.
        if let Some(text) = run("netstat", &["-rn"]) {
            return gateway_from_netstat(&text);
        }
    }

    None
}

/// Run a helper quietly and hand back its stdout. Any failure — missing command, non-zero exit, no
/// output — is simply `None`: a gateway hint is worth trying for, not worth complaining about.
#[cfg(not(windows))]
fn run(exe: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(exe)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Windows: the console window of a helper must not flash on every start.
#[cfg(windows)]
fn run_hidden(exe: &str, args: &[&str]) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000);
    let out = cmd.output().ok()?;
    if out.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Is this token shaped like an IPv4 address? Loose on purpose — the point is to skip prose and
/// column headings, not to validate a packet header.
fn looks_like_ipv4(tok: &str) -> bool {
    tok.matches('.').count() == 3
        && !tok.is_empty()
        && tok.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Could this address actually be a gateway? A route row also carries `0.0.0.0` as destination and
/// netmask, and Windows writes the word `On-link` (or `Onlink`, depending on its build) where a
/// directly-connected route has no gateway. Only a real, routable address counts.
fn looks_like_gateway(tok: &str) -> bool {
    looks_like_ipv4(tok) && !tok.starts_with("0.") && !tok.starts_with("255.")
}

/// Windows `route print`: a row of the route table, starting with the destination `0.0.0.0`. The
/// columns are `Network / Netmask / Gateway / Interface / Metric`, but their width and separators
/// change with the display language, so the row is read as words: destination, netmask, gateway.
/// Anything that is not an address in the third column — Windows writes `On-link` or `Onlink` for a
/// route that goes out over an interface directly — means this row has no gateway.
#[cfg(windows)]
fn gateway_from_route_print(text: &str) -> Option<String> {
    for line in text.lines() {
        let l = line.trim();
        if !l.starts_with("0.0.0.0") {
            continue;
        }
        let words: Vec<&str> = l.split_whitespace().collect();
        if words.len() < 3 {
            continue;
        }
        // The first two columns are destination and netmask, both 0.0.0.0 on this row.
        if !looks_like_ipv4(words[0]) || !looks_like_ipv4(words[1]) {
            continue;
        }
        if looks_like_gateway(words[2]) {
            return Some(words[2].to_string());
        }
    }
    None
}

/// Windows `ipconfig`: the line labelled "Default Gateway" / "Standardgateway" / "Standartgateway".
#[cfg(windows)]
fn gateway_from_ipconfig(text: &str) -> Option<String> {
    for line in text.lines() {
        let after_colon = match line.split_once(':') {
            Some((label, rest)) => {
                // `contains` on a lower-cased label covers the English "Default Gateway" as well as
                // the German spellings, including the older "Standartgateway".
                let label = label.trim().to_ascii_lowercase();
                if label.contains("gateway") {
                    rest
                } else {
                    continue;
                }
            }
            None => continue,
        };
        // The address may sit behind a second colon ("IPv4-Adresse . . . : 192.168.1.2"), and the
        // label may carry dotted filler, so scan for the first IPv4-looking token.
        if let Some(tok) = after_colon.split_whitespace().find(|t| looks_like_ipv4(t)) {
            return Some(tok.to_string());
        }
    }
    None
}

/// Linux `ip route show`: `default via 192.168.178.1 dev wlan0 …`.
#[cfg(target_os = "linux")]
fn gateway_from_ip_route(text: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() != Some("default") {
            continue;
        }
        // `default dev wlan0` (a link-local default with no gateway) has nothing to offer, and
        // several default lines can exist; the first real `via` wins.
        let mut seen_via = false;
        for p in parts {
            if seen_via && looks_like_ipv4(p) {
                return Some(p.to_string());
            }
            seen_via = p == "via";
        }
    }
    None
}

/// `netstat -rn` (Linux and macOS): the row whose destination is `0.0.0.0` or `default`. Column
/// order is `Destination Gateway Genmask …`, so the second field is the answer — except that a
/// directly-connected route repeats the destination there, and macOS writes `link4`/`link1` for the
/// same idea. Neither is a gateway, which is exactly what [`looks_like_gateway`] rejects.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn gateway_from_netstat(text: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let dest = match parts.next() {
            Some(d) => d,
            None => continue,
        };
        if dest != "0.0.0.0" && dest != "default" {
            continue;
        }
        match parts.next() {
            Some(gw) if looks_like_gateway(gw) => return Some(gw.to_string()),
            _ => continue,
        }
    }
    None
}

/// macOS `route -n get default`: lines of the form `gateway: 192.168.1.1`.
#[cfg(target_os = "macos")]
fn gateway_from_colon_lines(text: &str) -> Option<String> {
    for line in text.lines() {
        let (key, value) = match line.split_once(':') {
            Some(kv) => kv,
            None => continue,
        };
        if key.trim() == "gateway" {
            let value = value.trim();
            if looks_like_ipv4(value) {
                return Some(value.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real machine's answer, if it has one. Whatever comes back has to be a usable address —
    /// the app puts it straight into a target's host field.
    #[test]
    fn gateway_hint_is_an_address_when_it_exists() {
        if let Some(g) = default_gateway() {
            assert!(looks_like_ipv4(&g), "not an IPv4 address: {g:?}");
        }
    }

    /// Parsing is covered on captured output so it holds for systems this test run is not on.
    /// The gateway rule itself is checked here as well, because the parsers that use it only exist
    /// on their own system while this rule is shared by all of them.
    #[test]
    fn only_routable_addresses_count_as_gateway() {
        assert!(looks_like_gateway("192.168.178.1"));
        assert!(!looks_like_gateway("0.0.0.0"), "destination column");
        assert!(!looks_like_gateway("255.255.255.0"), "netmask column");
        assert!(!looks_like_gateway("On-link"), "not an address");
        assert!(!looks_like_gateway("192.168.178.4a"));
    }

    /// Parsing is covered on captured output so it holds for systems this test run is not on.
    #[cfg(windows)]
    #[test]
    fn parses_windows_route_print() {
        let text = "
===========================================================================
Interface list
19...00 50 56 c0 00 ................VMware Virtual Ethernet Adapter
23...d8 5e d3 1a 4b ................Realtek PCIe GbE Family Controller

Active Routes:
Network Destination        Netmask          Gateway       Interface  Metric
          0.0.0.0          0.0.0.0      192.168.178.1     192.168.178.42     35
         127.0.0.0        255.0.0.0         On-link         127.0.0.1    331
";
        assert_eq!(
            gateway_from_route_print(text).as_deref(),
            Some("192.168.178.1")
        );
    }

    /// A row that names no gateway (`On-link`) must not produce one. Windows spells the same idea
    /// `On-link` in English and `Onlink` in German builds, so neither may be read as an address.
    #[cfg(windows)]
    #[test]
    fn windows_route_without_gateway_is_none() {
        for text in [
            "          0.0.0.0          0.0.0.0      On-link     192.168.178.42     35\n",
            "          0.0.0.0          0.0.0.0      Onlink      192.168.178.42     35\n",
        ] {
            assert_eq!(gateway_from_route_print(text), None, "{text}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn parses_windows_ipconfig_in_both_languages() {
        let en = "
   Ethernet adapter Ethernet:
      IPv4 Address. . . . . . . . . . . : 192.168.1.42(Preferred)
      Subnet Mask . . . . . . . . . . . : 255.255.255.0
      Default Gateway . . . . . . . . . : 192.168.1.1
";
        let de = "
   Ethernetadapter Ethernet:
      IPv4-Adresse  . . . . . . . . . . : 192.168.1.42(Bevorzugt)
      Standardgateway . . . . . . . . . : 10.0.0.138
";
        assert_eq!(gateway_from_ipconfig(en).as_deref(), Some("192.168.1.1"));
        assert_eq!(gateway_from_ipconfig(de).as_deref(), Some("10.0.0.138"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_linux_ip_route() {
        let text = "default via 192.168.178.1 dev wlan0 proto dhcp metric 600
169.254.0.0/16 dev wlan0 scope link metric 1000
";
        assert_eq!(
            gateway_from_ip_route(text).as_deref(),
            Some("192.168.178.1")
        );

        // A default with no `via` (point-to-point, or a container) has nothing to offer.
        assert_eq!(
            gateway_from_ip_route("default dev wlan0 scope link\n"),
            None
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn parses_netstat() {
        let text = "Kernel IP routing table
Destination     Gateway         Genmask         Flags Metric Ref    Use Iface
0.0.0.0         10.0.0.138      0.0.0.0         UG    600    0        0 eth0
10.0.0.0        0.0.0.0         255.255.255.0   U     100    0        0 eth0
";
        assert_eq!(gateway_from_netstat(text).as_deref(), Some("10.0.0.138"));

        // macOS spells the destination `default` and repeats it when there is no gateway.
        let mac = "Destination        Gateway            Flags               Netif Expire\ndefault            192.168.1.1        UGScg                 en0\n";
        assert_eq!(gateway_from_netstat(mac).as_deref(), Some("192.168.1.1"));

        // A directly-connected default names no router: macOS writes `link4` there, and some
        // BSD-flavoured tables repeat the destination instead. Neither may become a target.
        for text in [
            "Destination        Gateway            Flags               Netif Expire\ndefault            link4              USc                 en0\n",
            "0.0.0.0         0.0.0.0         0.0.0.0         UG      0    0        0 wlan0\n",
        ] {
            assert_eq!(gateway_from_netstat(text), None, "{text}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_macos_route_get() {
        let text = "    route to: default\ndestination: default\n       gateway: 192.168.1.1\n     interface: en0\n          flags: <UP,DONE,CLONING>\n recvpipe  sendpipe  ssthresh  rtt,msec    mtu        weight    expire\n       0         0         0         0      1500         1         0\n";
        assert_eq!(
            gateway_from_colon_lines(text).as_deref(),
            Some("192.168.1.1")
        );
    }

    /// The loose address test must not accept prose or a netmask-shaped word.
    #[test]
    fn ipv4_shape_test_is_loose_but_not_gullible() {
        assert!(looks_like_ipv4("192.168.178.1"));
        assert!(!looks_like_ipv4("Gateway"));
        assert!(!looks_like_ipv4("192.168.1"));
    }
}
