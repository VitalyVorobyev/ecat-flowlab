//! `flowlab tcp-bench`: TCP throughput with integrity checks, for TCP over EoE (or any path).
//!
//! `serve` answers a minimal HTTP/1.0, so `curl` works against it too:
//! `GET /bytes/N?seed=S` sends N bytes of the simulator's payload for seed S with an
//! `X-SHA256` header; `PUT` or `POST /upload` reads the body and answers
//! `BYTES SHA256 SECONDS`. `run` downloads and uploads through it, checks both SHA-256 and
//! writes the times (provenance `measured`).

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};
use sha2::{Digest, Sha256};

use crate::Result;

/// Largest download `serve` generates, bytes.
const MAX_DOWNLOAD: u64 = 1 << 30;
/// Bytes per read and write.
const CHUNK: usize = 1 << 16;

/// Options of `flowlab tcp-bench`.
#[derive(Args)]
pub(crate) struct TcpBenchArgs {
    #[command(subcommand)]
    cmd: TcpBenchCmd,
}

#[derive(Subcommand)]
enum TcpBenchCmd {
    /// Serve downloads and take uploads until Ctrl-C (one thread per connection).
    Serve {
        /// Address to listen on.
        #[arg(long, default_value = "0.0.0.0:8000", value_name = "ADDR")]
        listen: String,
    },
    /// Download and upload through a `tcp-bench serve`, check the SHA-256 of both and print
    /// the throughput.
    Run {
        /// Address of the server, e.g. 127.0.0.1:18000 (a gateway's forwarded port).
        #[arg(long, value_name = "HOST:PORT")]
        to: String,
        /// Bytes to download (0: none); a `k` or `M` suffix means KiB or MiB.
        #[arg(long, default_value = "1M", value_parser = parse_size, value_name = "BYTES")]
        download: u64,
        /// Bytes to upload (0: none).
        #[arg(long, default_value = "1M", value_parser = parse_size, value_name = "BYTES")]
        upload: u64,
        /// Seed of the payload (the upload uses seed + 1).
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Give up on a transfer after this long, seconds.
        #[arg(long, default_value_t = 300.0, value_name = "SECONDS")]
        timeout: f64,
        /// Write the result as JSON.
        #[arg(short, long, value_name = "JSON")]
        out: Option<PathBuf>,
        /// Append one row per direction to this CSV (with a header if it is new).
        #[arg(long, value_name = "CSV", requires = "name")]
        csv: Option<PathBuf>,
        /// Name of the run in the CSV.
        #[arg(long)]
        name: Option<String>,
    },
}

fn parse_size(s: &str) -> std::result::Result<u64, String> {
    let (num, mul) = match s.as_bytes().last() {
        Some(b'k' | b'K') => (&s[..s.len() - 1], 1024),
        Some(b'M') => (&s[..s.len() - 1], 1024 * 1024),
        _ => (s, 1),
    };
    num.parse::<u64>().map(|n| n * mul).map_err(|_| format!("`{s}` is not a size (bytes, or with k or M)"))
}

pub(crate) fn run(a: TcpBenchArgs) -> Result<()> {
    match a.cmd {
        TcpBenchCmd::Serve { listen } => {
            let l = TcpListener::bind(&listen).map_err(|e| format!("listen on {listen}: {e}"))?;
            eprintln!("tcp-bench serving on {}: GET /bytes/N?seed=S, PUT /upload. Ctrl-C stops.", l.local_addr()?);
            serve(l)
        }
        TcpBenchCmd::Run { to, download, upload, seed, timeout, out, csv, name } => {
            if !(timeout.is_finite() && timeout > 0.0) {
                return Err("--timeout must be positive".into());
            }
            let r = bench(&to, download, upload, seed, Duration::from_secs_f64(timeout))?;
            for d in [&r.download, &r.upload].into_iter().flatten() {
                eprintln!(
                    "{:<8} {:>10} B in {:>8.3} s: {:>9.1} kB/s, SHA-256 {}",
                    d.direction,
                    d.bytes,
                    d.seconds,
                    d.bytes_per_s() / 1000.0,
                    if d.sha256_ok { "ok" } else { "MISMATCH" }
                );
            }
            if let Some(p) = out {
                crate::ensure_parent(&p)?;
                std::fs::write(&p, serde_json::to_string_pretty(&r.to_json(&to))?)
                    .map_err(|e| format!("{}: {e}", p.display()))?;
            }
            if let (Some(p), Some(n)) = (csv, name) {
                append_csv(&p, &n, &r)?;
            }
            if [&r.download, &r.upload].into_iter().flatten().any(|d| !d.sha256_ok) {
                return Err("a transfer arrived with the wrong SHA-256".into());
            }
            Ok(())
        }
    }
}

/// Serve every connection on `l` in its own thread, forever.
pub(crate) fn serve(l: TcpListener) -> Result<()> {
    for conn in l.incoming() {
        let Ok(s) = conn else { continue };
        std::thread::spawn(move || {
            if let Err(e) = handle(s) {
                eprintln!("tcp-bench: {e}");
            }
        });
    }
    Ok(())
}

/// A request line and headers, as far as tcp-bench needs them.
struct Request {
    method: String,
    path: String,
    content_length: u64,
}

fn read_head(r: &mut impl BufRead) -> Result<(String, Vec<(String, String)>)> {
    let mut first = String::new();
    r.read_line(&mut first)?;
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Err("connection closed in the headers".into());
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
        if headers.len() > 100 {
            return Err("too many headers".into());
        }
    }
    Ok((first.trim_end().to_string(), headers))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn handle(s: TcpStream) -> Result<()> {
    s.set_nodelay(true)?;
    let mut r = BufReader::new(s.try_clone()?);
    let (first, headers) = read_head(&mut r)?;
    let mut parts = first.split_whitespace();
    let req = Request {
        method: parts.next().unwrap_or_default().to_string(),
        path: parts.next().unwrap_or_default().to_string(),
        content_length: header(&headers, "content-length").and_then(|v| v.parse().ok()).unwrap_or(0),
    };
    let mut w = s;
    match (req.method.as_str(), req.path.split_once('?').map_or(req.path.as_str(), |(p, _)| p)) {
        ("GET", p) if p.starts_with("/bytes/") => {
            let n: u64 = p["/bytes/".len()..].parse().map_err(|_| format!("bad size in {}", req.path))?;
            if n > MAX_DOWNLOAD {
                return respond(&mut w, "413 Too Large", b"at most 1 GiB\n");
            }
            let seed = req
                .path
                .split_once('?')
                .and_then(|(_, q)| q.split('&').find_map(|kv| kv.strip_prefix("seed=")))
                .and_then(|v| v.parse().ok())
                .unwrap_or(1);
            let sha = flowlab_mailbox::payload_sha256(seed, n);
            write!(
                w,
                "HTTP/1.0 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {n}\r\nX-SHA256: {sha}\r\n\r\n"
            )?;
            let mut buf = vec![0u8; CHUNK];
            let mut at = 0u64;
            while at < n {
                let k = CHUNK.min((n - at) as usize);
                flowlab_mailbox::payload_bytes(seed, at, &mut buf[..k]);
                w.write_all(&buf[..k])?;
                at += k as u64;
            }
            Ok(())
        }
        ("PUT" | "POST", "/upload") => {
            let t0 = Instant::now();
            let (got, sha) = hash_body(&mut r, req.content_length)?;
            let body = format!("{got} {sha} {:.3}\n", t0.elapsed().as_secs_f64());
            respond(&mut w, "200 OK", body.as_bytes())
        }
        _ => respond(&mut w, "404 Not Found", b"GET /bytes/N?seed=S or PUT /upload\n"),
    }
}

fn respond(w: &mut impl Write, status: &str, body: &[u8]) -> Result<()> {
    write!(w, "HTTP/1.0 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n", body.len())?;
    w.write_all(body)?;
    Ok(())
}

/// Read up to `len` bytes, hashing them: (bytes read, SHA-256).
fn hash_body(r: &mut impl Read, len: u64) -> Result<(u64, String)> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut got = 0u64;
    while got < len {
        let k = r.read(&mut buf[..CHUNK.min((len - got) as usize)])?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        got += k as u64;
    }
    Ok((got, flowlab_core::hex(&h.finalize())))
}

/// One direction of a run.
#[derive(Debug, Clone)]
pub(crate) struct Leg {
    direction: &'static str,
    http_status: u16,
    bytes: u64,
    /// From connecting to the last byte, as the client sees it.
    seconds: f64,
    /// Download: time to the first byte of the body; upload: the server's receive time.
    first_byte_or_server_s: f64,
    sha256_ok: bool,
}

impl Leg {
    fn bytes_per_s(&self) -> f64 {
        if self.seconds > 0.0 { self.bytes as f64 / self.seconds } else { 0.0 }
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "http_status": self.http_status,
            "bytes": self.bytes,
            "seconds": self.seconds,
            if self.direction == "download" { "first_byte_s" } else { "server_s" }: self.first_byte_or_server_s,
            "bytes_per_s": self.bytes_per_s(),
            "sha256_ok": self.sha256_ok,
        })
    }
}

/// A download and an upload.
#[derive(Debug, Clone)]
pub(crate) struct BenchResult {
    seed: u64,
    download: Option<Leg>,
    upload: Option<Leg>,
}

impl BenchResult {
    fn to_json(&self, to: &str) -> serde_json::Value {
        serde_json::json!({
            "provenance": flowlab_core::Provenance::Measured,
            "tool": "flowlab tcp-bench",
            "to": to,
            "seed": self.seed,
            "download": self.download.as_ref().map(Leg::to_json),
            "upload": self.upload.as_ref().map(Leg::to_json),
        })
    }
}

fn connect(to: &str, timeout: Duration) -> Result<TcpStream> {
    let addr = to.to_socket_addrs()?.next().ok_or_else(|| format!("{to}: no address"))?;
    let s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("connect to {to}: {e}"))?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    s.set_nodelay(true)?;
    Ok(s)
}

fn status_code(first: &str) -> u16 {
    first.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
}

/// Download `download` and upload `upload` bytes through the server at `to`.
pub(crate) fn bench(to: &str, download: u64, upload: u64, seed: u64, timeout: Duration) -> Result<BenchResult> {
    let mut result = BenchResult { seed, download: None, upload: None };
    if download > 0 {
        let t0 = Instant::now();
        let mut s = connect(to, timeout)?;
        write!(s, "GET /bytes/{download}?seed={seed} HTTP/1.0\r\nHost: {to}\r\n\r\n")?;
        let mut r = BufReader::new(s);
        let (first, headers) = read_head(&mut r)?;
        let ttfb = t0.elapsed().as_secs_f64();
        let want = header(&headers, "x-sha256").unwrap_or_default().to_string();
        let (got, sha) = hash_body(&mut r, download)?;
        let seconds = t0.elapsed().as_secs_f64();
        let ok = got == download && sha == want && want == flowlab_mailbox::payload_sha256(seed, download);
        result.download = Some(Leg {
            direction: "download",
            http_status: status_code(&first),
            bytes: got,
            seconds,
            first_byte_or_server_s: ttfb,
            sha256_ok: ok,
        });
    }
    if upload > 0 {
        let useed = seed.wrapping_add(1);
        let t0 = Instant::now();
        let mut s = connect(to, timeout)?;
        write!(
            s,
            "PUT /upload HTTP/1.0\r\nHost: {to}\r\nContent-Type: application/octet-stream\r\nContent-Length: {upload}\r\n\r\n"
        )?;
        let mut buf = vec![0u8; CHUNK];
        let mut at = 0u64;
        while at < upload {
            let k = CHUNK.min((upload - at) as usize);
            flowlab_mailbox::payload_bytes(useed, at, &mut buf[..k]);
            s.write_all(&buf[..k])?;
            at += k as u64;
        }
        let mut r = BufReader::new(s);
        let (first, _) = read_head(&mut r)?;
        let mut body = String::new();
        r.read_to_string(&mut body)?;
        let seconds = t0.elapsed().as_secs_f64();
        let mut f = body.split_whitespace();
        let (n, sha, server_s) = (f.next(), f.next(), f.next().and_then(|v| v.parse().ok()).unwrap_or(0.0));
        let ok = n.and_then(|v| v.parse::<u64>().ok()) == Some(upload)
            && sha == Some(flowlab_mailbox::payload_sha256(useed, upload).as_str());
        result.upload = Some(Leg {
            direction: "upload",
            http_status: status_code(&first),
            bytes: upload,
            seconds,
            first_byte_or_server_s: server_s,
            sha256_ok: ok,
        });
    }
    Ok(result)
}

fn append_csv(path: &PathBuf, name: &str, r: &BenchResult) -> Result<()> {
    crate::ensure_parent(path)?;
    let new = !path.exists();
    let mut f =
        OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if new {
        writeln!(f, "name,direction,http,bytes,seconds,ttfb_or_server_s,bytes_per_s,sha")?;
    }
    for d in [&r.download, &r.upload].into_iter().flatten() {
        writeln!(
            f,
            "{name},{},{},{},{:.6},{:.6},{:.0},{}",
            d.direction,
            d.http_status,
            d.bytes,
            d.seconds,
            d.first_byte_or_server_s,
            d.bytes_per_s(),
            if d.sha256_ok { "ok" } else { "MISMATCH" }
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loopback_run_checks_both_directions() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let to = l.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            let _ = serve(l);
        });
        let r = bench(&to, 300_001, 200_003, 5, Duration::from_secs(10)).unwrap();
        let (d, u) = (r.download.unwrap(), r.upload.unwrap());
        assert!(d.sha256_ok && u.sha256_ok, "{d:?} {u:?}");
        assert_eq!((d.http_status, d.bytes, u.http_status, u.bytes), (200, 300_001, 200, 200_003));
        let csv = std::env::temp_dir().join(format!("flowlab-tcp-bench-{}.csv", std::process::id()));
        let _ = std::fs::remove_file(&csv);
        let r = bench(&to, 1000, 0, 5, Duration::from_secs(10)).unwrap();
        append_csv(&csv, "t", &r).unwrap();
        let text = std::fs::read_to_string(&csv).unwrap();
        std::fs::remove_file(&csv).unwrap();
        assert!(text.starts_with("name,direction,http,bytes,") && text.contains("\nt,download,200,1000,"), "{text}");
    }

    #[test]
    fn unknown_paths_get_404_and_sizes_parse() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let to = l.local_addr().unwrap();
        std::thread::spawn(move || {
            let _ = serve(l);
        });
        let mut s = TcpStream::connect(to).unwrap();
        s.write_all(b"GET /nothing HTTP/1.0\r\n\r\n").unwrap();
        let mut back = String::new();
        s.read_to_string(&mut back).unwrap();
        assert!(back.starts_with("HTTP/1.0 404"), "{back}");
        assert_eq!(parse_size("2M"), Ok(2 << 20));
        assert_eq!(parse_size("512k"), Ok(512 << 10));
        assert!(parse_size("1G").is_err());
    }
}
