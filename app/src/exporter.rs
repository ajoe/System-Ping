//! CSV export of the recorded samples, and the optional protocol file.

use crate::history::{self, Fail, Series};
use std::io::Write;

/// Write one CSV file with `timestamp,target,latency_ms,status`.
/// Returns Err(text) so the UI can show why it failed.
pub fn write_csv(path: &std::path::Path, rows: &[Row]) -> Result<usize, String> {
    let mut buf = Vec::with_capacity(rows.len() * 48 + 64);
    buf.extend_from_slice(b"timestamp,target,latency_ms,status\n");
    for r in rows {
        let field = |buf: &mut Vec<u8>, s: &str| {
            buf.push(b'"');
            for c in s.chars() {
                match c {
                    '"' => buf.extend_from_slice(b"\"\""),
                    '\n' => buf.extend_from_slice(b"\\n"),
                    '\r' => {}
                    _ => {
                        let mut tmp = [0u8; 4];
                        buf.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
                    }
                }
            }
            buf.push(b'"');
        };
        field(&mut buf, &r.timestamp);
        buf.push(b',');
        field(&mut buf, &r.target);
        buf.push(b',');
        match r.latency_ms {
            Some(ms) => {
                let s = format!("{ms:.1}");
                buf.extend_from_slice(s.as_bytes());
            }
            None => buf.extend_from_slice(b"\"\""),
        }
        buf.push(b',');
        field(&mut buf, &r.status);
        buf.push(b'\n');
    }
    std::fs::write(path, &buf).map_err(|e| format!("{}: {}", path.display(), e))?;
    // The caller reports how many rows landed on disk, not the byte count.
    Ok(rows.len())
}

/// How a protocol file is written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProtocolFormat {
    /// Comma separated, one line per measurement — same fields as the CSV export.
    #[default]
    Csv,
    /// One readable line per measurement: `time  target  latency`.
    Text,
}

impl ProtocolFormat {
    /// Pick by file ending: `.txt`/`.log` give text, everything else CSV.
    pub fn from_path(path: &str) -> Self {
        let p = path.trim().to_ascii_lowercase();
        if p.ends_with(".txt") || p.ends_with(".log") {
            ProtocolFormat::Text
        } else {
            ProtocolFormat::Csv
        }
    }
}

/// True for a reply; anything negative is a lost probe.
fn is_ok(rtt_ms: f32) -> bool {
    rtt_ms >= 0.0
}

/// Build the export rows for one target's history, oldest first.
pub fn collect_rows(
    label: &str,
    series: &Series,
    fallback_origin: chrono::DateTime<chrono::Local>,
) -> Vec<Row> {
    let mut rows = Vec::with_capacity(series.len());
    for s in series.samples() {
        // Each sample carries its own absolute time. `fallback_origin` is only used for a
        // sample that predates that field (a hand-built series from an older caller).
        let at = match chrono::DateTime::from_timestamp_millis(s.at_ms) {
            Some(utc) => utc.with_timezone(&chrono::Local),
            None => fallback_origin + chrono::Duration::milliseconds((s.t_secs * 1000.0) as i64),
        };
        rows.push(Row::from_sample(label, at, s));
    }
    rows
}

/// One exported measurement. `latency_ms` is None for a lost probe.
#[derive(Clone, Debug)]
pub struct Row {
    pub timestamp: String,
    pub target: String,
    pub latency_ms: Option<f32>,
    pub status: String,
}

impl Row {
    /// Build a row from one sample and the wall-clock time it belongs to.
    pub fn from_sample(
        target: &str,
        at: chrono::DateTime<chrono::Local>,
        s: &history::Sample,
    ) -> Self {
        Self {
            timestamp: at.to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
            target: target.to_string(),
            latency_ms: if is_ok(s.rtt_ms) {
                Some(s.rtt_ms)
            } else {
                None
            },
            status: if is_ok(s.rtt_ms) {
                String::from("ok")
            } else {
                s.fail.map(Fail::short).unwrap_or("timeout").to_string()
            },
        }
    }

    fn line(&self) -> String {
        let latency = match self.latency_ms {
            Some(ms) => format!("{ms:.1}"),
            None => String::new(),
        };
        format!(
            "{},{},{},{}",
            csv_field(&self.timestamp),
            csv_field(&self.target),
            latency,
            csv_field(&self.status)
        )
    }

    /// The same fields as a plain text line: `2025-01-01 12:00:00.123  target  4 ms`.
    fn text_line(&self) -> String {
        let latency = match self.latency_ms {
            Some(ms) => format!("{} ms", fmt_ms(ms)),
            None => self.status.clone(),
        };
        format!("{}  {}  {}", self.timestamp, self.target, latency)
    }
}

/// Quote a CSV field the way Excel expects.
fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Whole milliseconds in the text protocol, so a line reads "4 ms" and not "4.0 ms".
fn fmt_ms(ms: f32) -> u32 {
    if !ms.is_finite() || ms <= 0.0 {
        0
    } else {
        (ms + 0.5) as u32
    }
}

/// Appends measurements to a protocol file as they arrive.
///
/// The file is opened once and kept open (append mode), so the lines stream out while the
/// app runs instead of being rewritten at the end. A write error is reported once and then
/// ignored: a full or unplugged disk must not stop the probing.
pub struct ProtocolFile {
    path: std::path::PathBuf,
    format: ProtocolFormat,
    file: Option<std::fs::File>,
    /// Set after the first failure so the message is shown once, not every probe.
    pub failed: Option<String>,
}

impl ProtocolFile {
    pub fn new(path: std::path::PathBuf, format: ProtocolFormat) -> Self {
        let mut out = Self {
            path,
            format,
            file: None,
            failed: None,
        };
        out.open();
        out
    }

    /// (Re)open the file in append mode and write a header when the file was empty.
    pub fn open(&mut self) {
        if self.file.is_some() {
            return;
        }
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = std::fs::create_dir_all(parent);
            }
        }
        let fresh = !self.path.exists();
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(f) => {
                self.file = Some(f);
                self.failed = None;
                if fresh {
                    let header = match self.format {
                        ProtocolFormat::Csv => "timestamp,target,latency_ms,status\n",
                        ProtocolFormat::Text => "# Ping-Protokoll: Zeit, Ziel, Antwortzeit\n",
                    };
                    let _ = writeln!(
                        self.file.as_mut().unwrap(),
                        "{}",
                        header.trim_end_matches('\n')
                    );
                }
            }
            Err(e) => self.failed = Some(format!("{}: {}", self.path.display(), e)),
        }
    }

    /// Append one measurement. Returns false once the file is unusable, so the caller can stop
    /// building rows.
    pub fn write(&mut self, row: &Row) -> bool {
        if self.file.is_none() {
            return false;
        }
        let line = match self.format {
            ProtocolFormat::Csv => row.line(),
            ProtocolFormat::Text => row.text_line(),
        };
        let file = match self.file.as_mut() {
            Some(f) => f,
            None => return false,
        };
        match writeln!(file, "{line}") {
            Ok(()) => true,
            Err(e) => {
                self.failed = Some(format!("{}: {}", self.path.display(), e));
                self.file = None;
                false
            }
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Whether the file is ready to take lines.
    pub fn is_open(&self) -> bool {
        self.file.is_some()
    }

    /// Forget the last write error (used after a successful reopen).
    pub fn clear_failed(&mut self) {
        self.failed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_field_quotes_only_when_needed() {
        assert_eq!(csv_field("google"), "google");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn protocol_appends_and_keeps_earlier_lines() {
        let dir = std::env::temp_dir().join(format!("system-ping-proto-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("protocol.csv");
        let _ = std::fs::remove_file(&path);

        let row = |ms: f32| Row {
            timestamp: String::from("2025-01-01T12:00:00.000+01:00"),
            target: String::from("loopback"),
            latency_ms: Some(ms),
            status: String::from("ok"),
        };

        {
            let mut p = ProtocolFile::new(path.clone(), ProtocolFormat::Csv);
            assert!(p.write(&row(4.0)));
        }
        // Reopening must append, not rewrite.
        {
            let mut p = ProtocolFile::new(path.clone(), ProtocolFormat::Csv);
            assert!(p.write(&row(9.0)));
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "timestamp,target,latency_ms,status");
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines[1].ends_with(",4.0,ok"), "{}", lines[1]);
        assert!(lines[2].ends_with(",9.0,ok"), "{}", lines[2]);

        let mut p = ProtocolFile::new(path.clone(), ProtocolFormat::Csv);
        p.file = None;
        assert!(!p.write(&row(1.0)), "a closed file reports failure");

        let _ = std::fs::remove_file(&path);
    }
}
