// Bonjour (mDNS) discovery through macOS's own `dns-sd` tool. adb's built-in mDNS
// browser misses services that mDNSResponder sees (e.g. a phone in `adb tcpip` mode).
use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, ToSocketAddrs};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Runs `dns-sd` (which never exits on its own) and collects stdout lines until
/// `wait` elapses or `done` returns true for a line.
fn dns_sd_lines(args: &[&str], wait: Duration, done: impl Fn(&str) -> bool) -> Vec<String> {
    let Ok(mut child) = Command::new("/usr/bin/dns-sd")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let (tx, rx) = mpsc::channel();
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    let deadline = Instant::now() + wait;
    let mut lines = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(line) => {
                let finished = done(&line);
                lines.push(line);
                if finished {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    lines
}

/// Browse line: "<time>  Add  <flags> <if> <domain> <service type> <instance name>"
fn parse_browse_line(line: &str) -> Option<String> {
    let cols: Vec<&str> = line.split_whitespace().collect();
    (cols.len() >= 7 && cols[1] == "Add").then(|| cols[6..].join(" "))
}

/// Resolve line: "... can be reached at <host>.:<port> (interface N)"
fn parse_resolve_line(line: &str) -> Option<(String, u16)> {
    let target = line.split(" can be reached at ").nth(1)?.split_whitespace().next()?;
    let (host, port) = target.rsplit_once(':')?;
    Some((host.trim_end_matches('.').to_string(), port.parse().ok()?))
}

/// Instance names currently advertised for `service_type` (e.g. "_adb._tcp").
pub fn browse(service_type: &str, wait: Duration) -> Vec<String> {
    let mut names: Vec<String> = dns_sd_lines(&["-B", service_type, "local."], wait, |_| false)
        .iter()
        .filter_map(|l| parse_browse_line(l))
        .collect();
    names.dedup();
    names
}

/// Resolves a service instance to an IPv4 socket address.
pub fn resolve(instance: &str, service_type: &str, wait: Duration) -> Option<SocketAddr> {
    let lines = dns_sd_lines(&["-L", instance, service_type, "local."], wait, |l| {
        parse_resolve_line(l).is_some()
    });
    let (host, port) = lines.iter().find_map(|l| parse_resolve_line(l))?;
    // getaddrinfo resolves ".local" names through mDNSResponder on macOS
    (host.as_str(), port).to_socket_addrs().ok()?.find(SocketAddr::is_ipv4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_browse_add_lines_only() {
        let add = "23:06:51.428  Add        2  11 local.               _adb._tcp.           adb-056371405R100461";
        let rmv = "23:06:51.428  Rmv        0  11 local.               _adb._tcp.           adb-056371405R100461";
        assert_eq!(parse_browse_line(add).as_deref(), Some("adb-056371405R100461"));
        assert_eq!(parse_browse_line(rmv), None);
        assert_eq!(parse_browse_line("Timestamp     A/R    Flags  if Domain"), None);
    }

    #[test]
    fn parses_resolve_target() {
        let line = "23:06:59.075  adb-0563._adb._tcp.local. can be reached at Android.local.:5555 (interface 11)";
        assert_eq!(parse_resolve_line(line), Some(("Android.local".to_string(), 5555)));
        assert_eq!(parse_resolve_line("Lookup adb-0563._adb._tcp.local"), None);
    }
}
