//! Single-owner snapshot server. Put a TLS reverse proxy in front of the loopback listener.

use anyhow::{Context, Result, bail};
use std::{
    fs::{self, File},
    io::Read,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tiny_http::{Header, Method, Response, Server, StatusCode};

const MAX_BYTES: u64 = leetdesk::vault::MAX_ARCHIVE_BYTES;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn main() {
    if let Err(error) = run() {
        eprintln!("LeetDesk server: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "LeetDesk 单人备份服务\n环境变量：LEETDESK_SERVER_TOKEN（至少 32 字符），LEETDESK_SERVER_DATA（备份目录），LEETDESK_SERVER_BIND（默认 127.0.0.1:8787）"
        );
        return Ok(());
    }
    let token = std::env::var("LEETDESK_SERVER_TOKEN").context("请设置 LEETDESK_SERVER_TOKEN")?;
    if token.len() < 32 || !token.is_ascii() || token.bytes().any(|b| b.is_ascii_whitespace()) {
        bail!("LEETDESK_SERVER_TOKEN 须为至少 32 个无空白的 ASCII 字符");
    }
    let root = PathBuf::from(
        std::env::var_os("LEETDESK_SERVER_DATA").unwrap_or_else(|| "./leetdesk-server-data".into()),
    );
    fs::create_dir_all(root.join("snapshots"))?;
    let bind = std::env::var("LEETDESK_SERVER_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into());
    let server =
        Server::http(&bind).map_err(|error| anyhow::anyhow!("无法监听 {bind}: {error}"))?;
    println!("LeetDesk backup server listening on {bind}");
    for mut request in server.incoming_requests() {
        let authorized = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Authorization"))
            .and_then(|h| h.value.as_str().strip_prefix("Bearer "))
            .is_some_and(|candidate| constant_time_eq(candidate.as_bytes(), token.as_bytes()));
        if !authorized {
            let _ = request.respond(response("unauthorized", 401));
            continue;
        }
        let method = request.method().clone();
        let path = request.url().to_string();
        match (method, path.as_str()) {
            (Method::Get, "/v1/health") => {
                let _ = request.respond(response("ok", 200));
            }
            (Method::Post, "/v1/snapshots") => {
                let result = (|| -> Result<String> {
                    let dir = root.join("snapshots");
                    let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
                    let copied =
                        std::io::copy(&mut request.as_reader().take(MAX_BYTES + 1), &mut temp)?;
                    if copied == 0 || copied > MAX_BYTES {
                        bail!("备份包为空或超过大小限制");
                    }
                    // Reject malformed or unexpected archives before keeping a backup.
                    validate_archive(temp.path())?;
                    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
                    let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
                    let filename = format!("{stamp}-{seq}.zip");
                    temp.persist_noclobber(dir.join(&filename))?;
                    Ok(filename)
                })();
                let (message, status) = match result {
                    Ok(filename) => (filename, 201),
                    Err(error) => (format!("{error:#}"), 400),
                };
                let _ = request.respond(response(&message, status));
            }
            (Method::Get, "/v1/snapshots/latest") => {
                let latest = fs::read_dir(root.join("snapshots"))
                    .ok()
                    .and_then(|entries| {
                        entries
                            .filter_map(|e| e.ok().map(|e| e.path()))
                            .filter(|p| p.extension().is_some_and(|e| e == "zip"))
                            .max()
                    });
                match latest.and_then(|p| File::open(p).ok()) {
                    Some(file) => {
                        let mut reply = Response::from_file(file);
                        reply.add_header(
                            Header::from_bytes("Content-Type", "application/zip").unwrap(),
                        );
                        let _ = request.respond(reply);
                    }
                    None => {
                        let _ = request.respond(response("no snapshots", 404));
                    }
                }
            }
            _ => {
                let _ = request.respond(response("not found", 404));
            }
        }
    }
    Ok(())
}

fn validate_archive(path: &std::path::Path) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let store = leetdesk::store::Store::open(temp.path().join("verify"))?;
    leetdesk::vault::import(&store, path)?;
    Ok(())
}

fn response(message: &str, status: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(message.as_bytes().to_vec()).with_status_code(StatusCode(status))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (a.get(i).copied().unwrap_or_default() ^ b.get(i).copied().unwrap_or_default())
            as usize;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bearer_comparison_checks_all_bytes() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcx"));
    }
}
