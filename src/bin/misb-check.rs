//! misb-check: fails drone video whose KLV is missing or broken.
//!
//! ```text
//! misb-check [--format text|json|github] [--strict] [--max N] PATH...
//! ```
//!
//! PATH is a transport stream (.ts, .mpg, .m2ts), a raw KLV stream (the
//! output of `ffmpeg -map 0:d -c copy -f data`) or a directory, searched
//! for both. Exit status: 0 every file passed, 1 at least one file failed,
//! 2 a file could not be read or the arguments are wrong.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use misb::conform::{check_stream, Finding, Report, Severity};

const USAGE: &str = "usage: misb-check [--format text|json|github] [--strict] [--max N] PATH...

Checks the MISB ST 0601 KLV in drone video (STANAG 4609) and fails on:
missing KLV, missing mandatory tags (2, 65, 1), bad checksums, wrong
value sizes, ST 1201 values outside their mapping or disagreeing with
the older tag, time stamps that go backwards or were never set, and
positions that cannot be real.

PATH: .ts/.mpg/.m2ts video, a raw .klv stream, or a directory.
  --format  text (default), json, or github (workflow annotations)
  --strict  warnings fail the file too
  --max N   findings printed per file in text mode (default 20, 0 = all)
Exit: 0 pass, 1 fail, 2 unreadable input or bad arguments.";

const VIDEO_EXT: &[&str] = &["ts", "mts", "m2ts", "m2t", "mpg", "mpeg", "trp", "tp"];
const KLV_EXT: &[&str] = &["klv", "bin"];

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Text,
    Json,
    Github,
}

struct Checked {
    path: PathBuf,
    container: &'static str,
    /// (PID, report) per data stream; PID is None for a raw KLV file.
    streams: Vec<(Option<u16>, Report)>,
}

impl Checked {
    fn errors(&self) -> usize {
        self.streams.iter().map(|s| s.1.errors()).sum()
    }
    fn warnings(&self) -> usize {
        self.streams.iter().map(|s| s.1.warnings()).sum()
    }
    fn packets(&self) -> usize {
        self.streams.iter().map(|s| s.1.packets).sum()
    }
}

fn check_file(path: &Path) -> std::io::Result<Checked> {
    let bytes = std::fs::read(path)?;
    let raw_klv = bytes.starts_with(&misb::klv::UL_PREFIX);
    if !raw_klv && misb::ts::is_transport_stream(&bytes) {
        let mut streams: Vec<(Option<u16>, Report)> = misb::ts::klv_streams(&bytes)
            .into_iter()
            .map(|s| (Some(s.pid), check_stream(&s.data)))
            .filter(|(_, r)| r.packets > 0 || r.eg0104_units > 0)
            .collect();
        if streams.is_empty() {
            streams.push((None, check_stream(&[])));
        }
        return Ok(Checked {
            path: path.to_path_buf(),
            container: "mpeg-ts",
            streams,
        });
    }
    Ok(Checked {
        path: path.to_path_buf(),
        container: "klv",
        streams: vec![(None, check_stream(&bytes))],
    })
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out)?;
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_ascii_lowercase();
            if VIDEO_EXT.contains(&ext.as_str()) || KLV_EXT.contains(&ext.as_str()) {
                out.push(p);
            }
        }
    }
    Ok(())
}

fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or("null".into(), |v| v.to_string())
}

fn finding_json(f: &Finding, pid: Option<u16>) -> String {
    format!(
        "{{\"severity\":\"{}\",\"code\":\"{}\",\"pid\":{},\"packet\":{},\"offset\":{},\"tag\":{},\"message\":{}}}",
        f.severity,
        f.code,
        opt(pid),
        opt(f.packet),
        opt(f.offset),
        opt(f.tag),
        json_str(&f.message)
    )
}

fn fails(c: &Checked, strict: bool) -> bool {
    c.errors() > 0 || (strict && c.warnings() > 0)
}

fn print_text(c: &Checked, strict: bool, max: usize) {
    let verdict = if fails(c, strict) { "FAIL" } else { "PASS" };
    println!(
        "{}: {verdict} ({} errors, {} warnings, {} ST 0601 packets, {})",
        c.path.display(),
        c.errors(),
        c.warnings(),
        c.packets(),
        c.container
    );
    let mut shown = 0;
    let mut by_code: BTreeMap<&str, usize> = BTreeMap::new();
    for (pid, r) in &c.streams {
        for f in &r.findings {
            *by_code.entry(f.code).or_default() += 1;
            if max == 0 || shown < max {
                match pid {
                    Some(p) => println!("  PID {p:#x} {f}"),
                    None => println!("  {f}"),
                }
                shown += 1;
            }
        }
    }
    let total: usize = by_code.values().sum();
    if total > shown {
        let summary: Vec<String> = by_code.iter().map(|(k, v)| format!("{k} x{v}")).collect();
        println!(
            "  ... {} more; all findings: {}",
            total - shown,
            summary.join(", ")
        );
    }
}

fn print_github(c: &Checked) {
    // One annotation per code per file, so a long capture does not flood the run.
    let mut first: BTreeMap<&str, (&Finding, usize)> = BTreeMap::new();
    for (_, r) in &c.streams {
        for f in &r.findings {
            first.entry(f.code).or_insert((f, 0)).1 += 1;
        }
    }
    for (code, (f, n)) in first {
        let level = if f.severity == Severity::Error {
            "error"
        } else {
            "warning"
        };
        let more = if n > 1 {
            format!(" ({n} packets)")
        } else {
            String::new()
        };
        println!(
            "::{level} file={},title=misb {code}::{f}{more}",
            c.path.display()
        );
    }
}

fn main() -> ExitCode {
    let mut format = Format::Text;
    let mut strict = false;
    let mut max = 20usize;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("misb-check {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--strict" => strict = true,
            "--format" => {
                format = match args.next().as_deref() {
                    Some("text") => Format::Text,
                    Some("json") => Format::Json,
                    Some("github") => Format::Github,
                    _ => {
                        eprintln!("{USAGE}");
                        return ExitCode::from(2);
                    }
                }
            }
            "--max" => match args.next().and_then(|n| n.parse().ok()) {
                Some(n) => max = n,
                None => {
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            },
            s if s.starts_with('-') && s.len() > 1 => {
                eprintln!("unknown option {s}\n{USAGE}");
                return ExitCode::from(2);
            }
            _ => paths.push(PathBuf::from(a)),
        }
    }
    if paths.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let mut files = Vec::new();
    let mut io_error = false;
    for p in paths {
        if p.is_dir() {
            if let Err(e) = walk(&p, &mut files) {
                eprintln!("{}: {e}", p.display());
                io_error = true;
            }
        } else {
            files.push(p);
        }
    }
    let mut failed = 0usize;
    let mut json = Vec::new();
    for f in &files {
        let c = match check_file(f) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{}: {e}", f.display());
                io_error = true;
                continue;
            }
        };
        if fails(&c, strict) {
            failed += 1;
        }
        match format {
            Format::Text => print_text(&c, strict, max),
            Format::Github => {
                print_github(&c);
                print_text(&c, strict, max);
            }
            Format::Json => {
                let findings: Vec<String> = c
                    .streams
                    .iter()
                    .flat_map(|(pid, r)| r.findings.iter().map(move |f| finding_json(f, *pid)))
                    .collect();
                json.push(format!(
                    "{{\"file\":{},\"container\":\"{}\",\"passed\":{},\"packets\":{},\"errors\":{},\"warnings\":{},\"findings\":[{}]}}",
                    json_str(&c.path.display().to_string()),
                    c.container,
                    !fails(&c, strict),
                    c.packets(),
                    c.errors(),
                    c.warnings(),
                    findings.join(",")
                ));
            }
        }
    }
    if format == Format::Json {
        println!("[{}]", json.join(",\n"));
    } else if files.len() > 1 {
        println!("{} of {} files failed", failed, files.len());
    }
    if io_error {
        ExitCode::from(2)
    } else if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
