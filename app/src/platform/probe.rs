//! Running the operating system's `ping` and reading a round-trip time out of its text.
//!
//! The OS command is used on purpose: raw ICMP sockets need administrator rights, which a normal
//! double-click launch does not have — on macOS they need root as well, so the helper binary there
//! is the way to stay usable without privileges.
//!
//! What differs between systems is narrow and specific, so it is kept narrow and specific here:
//!
//! * the arguments that mean "send exactly one request, give up after N" — Windows counts that wait
//!   in milliseconds (`-w 1200`), Linux and macOS in whole seconds (`-W 2`);
//! * which word introduces the reply time. English `ping` prints `time=42ms`, German Windows prints
//!   `Zeit=14ms`. Rather than guess one language, every system asks for both words: a program that
//!   reads its own output is not affected by the machine's display language, and an extra word in
//!   the list costs nothing because it simply never matches.
//! * which phrases explain a missing reply, so a DNS failure can be told apart from a silent host.

use std::borrow::Cow;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Why a probe did not answer. The UI shows these next to the timeout so an outage looks different
/// from a blocked or unresolvable host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProbeFail {
    /// The host name did not resolve.
    Resolve,
    /// No reply within the timeout.
    Timeout,
    /// The OS ping could not be run or said something unexpected.
    Error,
}

/// One system's way of asking for a single probe. Built by [`plan`] from the job's timeout.
pub struct Plan {
    pub exe: String,
    pub args: Vec<String>,
    /// The time words to look for in the output, longest first so `Zeit=` wins over a stray `time`.
    pub markers: &'static [&'static str],
}

/// The reply-time words every system accepts. See the module docs for why this is not per OS.
const TIME_WORDS: &[&str] = &["time", "Zeit"];

/// Build the command line for one probe. `timeout_ms` is the whole budget for the probe; the wait
/// the OS is told must fit inside it, which on Linux and macOS means rounding *up* to whole seconds
/// and letting [`run`] cut the process off at its own deadline.
/// Build the command line for one probe. `timeout_ms` is the whole budget for the probe; the wait
/// the OS is told must fit inside it, which on Linux and macOS means rounding *up* to whole seconds
/// and letting [`run`] cut the process off at its own deadline. The command name may be empty,
/// which means "whatever `ping` this system has".
pub fn plan(exe: &str, host: &str, timeout_ms: u32) -> Plan {
    #[cfg(windows)]
    let args = {
        // `ping -n 1` sends one echo request; `-w` is the per-reply timeout in ms.
        vec![
            "-n".to_string(),
            "1".to_string(),
            "-w".to_string(),
            timeout_ms.to_string(),
            host.to_string(),
        ]
    };

    #[cfg(not(windows))]
    let args = {
        // `-c 1` is one request. `-W` is the wait for a reply, in whole seconds on both Linux and
        // macOS (the BSD and iputils spellings agree here); rounding up keeps a sub-second budget
        // from turning into "wait for nothing".
        let secs = ((timeout_ms as u64 + 999) / 1000).max(1).to_string();
        vec![
            "-c".to_string(),
            "1".to_string(),
            "-W".to_string(),
            secs,
            host.to_string(),
        ]
    };

    Plan {
        exe: if exe.is_empty() { "ping" } else { exe }.to_string(),
        args,
        markers: TIME_WORDS,
    }
}

/// Run the planned command and read the reply time. The caller has already decided the host is
/// safe to pass along (see [`crate::ping`]).
pub fn run(plan: &Plan, timeout_ms: u32) -> Result<f32, ProbeFail> {
    let mut cmd = Command::new(&plan.exe);
    cmd.args(&plan.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        // Keep the ping window from flashing on every probe.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return Err(ProbeFail::Error),
    };
    // A little grace beyond the OS's own wait: a ping that is still printing its summary when the
    // deadline passes must not be killed mid-line and read as a lost reply.
    let deadline = Instant::now() + Duration::from_millis(timeout_ms as u64 + 1500);
    let mut buf = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let mut sink = Vec::new();
        // Read with a hard deadline so a stuck ping can never hang the worker.
        loop {
            if Instant::now() > deadline {
                let _ = child.kill();
                break;
            }
            let mut chunk = [0u8; 512];
            match pipe.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => sink.extend_from_slice(&chunk[..n]),
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        // Windows ping prints in the OEM codepage; try UTF-8 then Latin-1. `from_utf8` consumes
        // the buffer, so the output is never copied twice. On Linux and macOS the output is UTF-8
        // already and the first branch takes it.
        match String::from_utf8(sink) {
            Ok(s) => buf = s,
            Err(e) => buf = e.into_bytes().iter().map(|&b| b as char).collect(),
        }
    }

    let _status = match child.wait() {
        Ok(s) => s,
        Err(_) => return Err(ProbeFail::Error),
    };
    // The exit code is deliberately dropped. Windows `ping.exe` exits 2 both when a host gives no
    // reply and when the name cannot be resolved, and the Unix pings exit non-zero on any loss —
    // neither can tell "did not resolve" from "stayed silent". The text can, and that is what
    // decides here.
    match parse_rtt(&buf, plan.markers) {
        Some(v) => Ok(v),
        None => Err(classify_failure(&buf)),
    }
}

/// Sort out why there was no reply time in the output. The phrases of all three systems are listed
/// together: this only ever runs on text from the local `ping`, so an extra phrase from another OS
/// cannot cause a wrong answer, and a machine whose locale does not match the app's UI language is
/// still classified correctly.
fn classify_failure(text: &str) -> ProbeFail {
    let t = text.to_ascii_lowercase();
    const RESOLVE: &[&str] = &[
        "could not find",
        "name or service",
        "unknown host",
        "unbekannter",
        "nicht gefunden",
        "failed to resolve",
        "no such host",
        // macOS and the BSD ping, and iputils on a nameless host.
        "cannot parse",
        "nodename nor servname",
        "name or service not known",
    ];
    const TIMEOUT: &[&str] = &[
        "timed out",
        "request timeout",
        "zeitüberschreitung",
        "zeitsberschreitung",
        "100% loss",
        "100 % verlust",
        "keine antwort",
        // iputils and macOS both print a summary line when nothing came back.
        "100% packet loss",
    ];
    if RESOLVE.iter().any(|m| t.contains(m)) {
        ProbeFail::Resolve
    } else if TIMEOUT.iter().any(|m| t.contains(m)) {
        ProbeFail::Timeout
    } else {
        ProbeFail::Error
    }
}

/// The value used for a reply that `ping` prints as `<1ms`. It has to be a positive number so the
/// chart and the loss statistics treat it as an answer, not a timeout.
pub const SUB_MS_RTT: f32 = 0.5;

/// Pull `time=42ms` / `time=1.2ms` (and the German `Zeit=…`) out of ping output.
///
/// The reply time is always printed as `<word><separator><number>ms`, but both parts differ by
/// language: English uses `time=42ms`, German `Zeit=14ms`, and a sub-millisecond reply becomes
/// `Zeit<1ms` / `time<1ms` with no digits of its own. So the scan looks for the time word and then
/// skips whatever separator follows it, instead of insisting on one exact spelling.
pub fn parse_rtt(text: &str, markers: &[&str]) -> Option<f32> {
    for line in text.lines() {
        for marker in markers {
            // Match the time word only: `time=` / `Zeit=` are both `<word><separator>`, and a
            // reply under a millisecond uses `<` instead of `=`, so the separator cannot be part
            // of the search string.
            let word = marker.trim_end_matches(['=', ' ']);
            let mut from = 0usize;
            // One reply line can carry the marker more than once; keep scanning so a marker in
            // front of prose never hides the number behind it.
            while let Some(rel) = line[from..].find(word) {
                let pos = from + rel + word.len();
                from = pos;
                // `Zeit<1ms`: a reply faster than a millisecond prints with a `<` where the digits
                // would start, so report "under one" rather than a lost packet.
                if line[pos..].starts_with('<') {
                    return Some(SUB_MS_RTT);
                }
                // Skip the separator (`=`, or nothing at all in some locales) before the number.
                let tail = &line[pos..].trim_start_matches(['=', ' ']);
                // Read the number straight out of the line instead of collecting it into a String
                // first; only a decimal comma needs replacing before parsing.
                let end = tail
                    .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
                    .unwrap_or(tail.len());
                let raw = tail[..end].trim_matches('.');
                if raw.is_empty() {
                    continue;
                }
                let cleaned = if raw.contains(',') {
                    Cow::Owned(raw.replace(',', "."))
                } else {
                    Cow::Borrowed(raw)
                };
                if let Ok(v) = cleaned.parse::<f32>() {
                    return Some(v);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reply line of an English Windows ping.
    #[test]
    fn parses_english_windows_time() {
        assert_eq!(
            parse_rtt("Reply from 1.1.1.1: time=42ms", &["time"]),
            Some(42.0)
        );
    }

    /// …and the German one, including the decimal comma.
    #[test]
    fn parses_german_windows_time() {
        assert_eq!(
            parse_rtt("Antwort von 8.8.8.8: Zeit=1,53ms", &["Zeit"]),
            Some(1.53)
        );
    }

    /// A reply under a millisecond has no digits of its own.
    #[test]
    fn parses_sub_millisecond() {
        assert_eq!(
            parse_rtt("Antwort von 127.0.0.1: Zeit<1ms TTL=128", &["Zeit"]),
            Some(0.5)
        );
        assert_eq!(parse_rtt("Reply from ::1: time<1ms", &["time"]), Some(0.5));
    }

    /// Linux (iputils) and macOS print the reply time the same way, so the same scan finds them —
    /// this is the line that made a Windows-only parser useless on the other two systems.
    #[test]
    fn parses_linux_and_macos_reply_lines() {
        let linux = "64 bytes from 8.8.8.8: icmp_seq=1 ttl=115 time=13.4 ms";
        assert_eq!(parse_rtt(linux, TIME_WORDS), Some(13.4));

        let macos = "64 bytes from 8.8.8.8: icmp_seq=0 ttl=112 time=27.905 ms";
        assert_eq!(parse_rtt(macos, TIME_WORDS), Some(27.905));

        // The summary line that follows carries `time` nowhere useful; the reply above still wins.
        let with_summary = format!("{linux}\n--- 8.8.8.8 ping statistics ---\n1 packets transmitted, 1 received, 0% packet loss");
        assert_eq!(parse_rtt(&with_summary, TIME_WORDS), Some(13.4));
    }

    /// A space between number and unit is the Unix spelling; a decimal with three digits stays
    /// exact because nothing rounds it here.
    #[test]
    fn parses_fractional_milliseconds() {
        assert_eq!(parse_rtt("time=0.041 ms", TIME_WORDS), Some(0.041));
    }

    /// No reply at all yields nothing rather than a made-up number.
    #[test]
    fn no_time_word_is_not_a_reply() {
        assert_eq!(parse_rtt("Request timed out.", &["time"]), None);
        assert_eq!(parse_rtt("", TIME_WORDS), None);
    }

    /// DNS failures on all three systems, in both languages.
    #[test]
    fn classifies_resolve_failures() {
        for text in [
            "Ping request could not find host foo.bar.",
            "ping: unknown host foo.bar",
            "ping: cannot parse foo.bar: Destination not found",
            "ping: foo.bar: Name or service not known",
            "ping: foo.bar: nodename nor servname provided, or not known",
        ] {
            assert_eq!(classify_failure(text), ProbeFail::Resolve, "{text}");
        }
    }

    /// Silent hosts on all three systems.
    #[test]
    fn classifies_timeouts() {
        for text in [
            "Request timed out.",
            "Antwort von 10.0.0.1: Zeitüberschreitung des Anfor...",
            "\n--- 10.0.0.1 ping statistics ---\n1 packets transmitted, 0 received, 100% packet loss",
            "100% loss",
        ] {
            assert_eq!(classify_failure(text), ProbeFail::Timeout, "{text}");
        }
    }

    /// Anything else stays an unexplained error instead of being guessed at.
    #[test]
    fn unknown_text_stays_an_error() {
        assert_eq!(classify_failure(""), ProbeFail::Error);
        assert_eq!(
            classify_failure("ping: permission denied"),
            ProbeFail::Error
        );
    }

    /// The argument shapes are what the systems actually speak: milliseconds on Windows, whole
    /// seconds elsewhere, and never a wait of zero.
    #[test]
    fn plans_one_request_per_system() {
        let p = plan("ping", "8.8.8.8", 1200);
        assert_eq!(p.exe, "ping");
        assert_eq!(p.args.last().unwrap(), "8.8.8.8");
        #[cfg(windows)]
        assert_eq!(p.args, ["-n", "1", "-w", "1200", "8.8.8.8"]);
        #[cfg(not(windows))]
        assert_eq!(p.args, ["-c", "1", "-W", "2", "8.8.8.8"]);

        // A sub-second budget must not round down to "wait for nothing". On Windows the wait is in
        // milliseconds and needs no rounding at all, so the same budget is simply passed through.
        let p = plan("ping", "8.8.8.8", 250);
        #[cfg(windows)]
        assert_eq!(&p.args[3], "250");
        #[cfg(not(windows))]
        assert_eq!(&p.args[3], "1");
    }

    /// An empty command name means "whatever the system has" rather than an executable named "".
    #[test]
    fn empty_exe_falls_back_to_ping() {
        assert_eq!(plan("", "8.8.8.8", 1000).exe, "ping");
    }
}
