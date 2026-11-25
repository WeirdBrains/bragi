//! Test and demo stand-ins for model management (LF-30): a mock Hugging Face Hub on a loopback
//! port and a `ModelsService` on a temp dir with a chosen memory budget. The tests and the demo
//! nodes use them; nothing touches the real Hub or a real node.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use weirdnet_agent::remote::Authorize;
use weirdnet_backends::models::ModelsService;
use weirdnet_backends::{Supervisor, SupervisorOptions};
use weirdnet_model::ModelStore;
use weirdnet_model::hw::Budget;

/// A repo on the mock Hub.
#[derive(Debug, Clone)]
pub struct MockRepo {
    pub id: String,
    pub downloads: u64,
    pub likes: u64,
    /// File name and size as the Hub lists it.
    pub files: Vec<(String, u64)>,
    /// Whether downloads work (the file is then `size` bytes of GGUF header and padding); big
    /// pretend repos only list sizes, so fit verdicts need no gigabytes.
    pub downloadable: bool,
}

impl MockRepo {
    pub fn new(id: &str, downloads: u64, likes: u64, files: &[(&str, u64)]) -> Self {
        Self { id: id.into(), downloads, likes, files: files.iter().map(|(n, s)| (n.to_string(), *s)).collect(), downloadable: false }
    }

    pub fn downloadable(mut self) -> Self {
        self.downloadable = true;
        self
    }
}

/// A GGUF header (llama, one block, one KV head of size 1) padded to `len` bytes.
pub fn gguf(len: usize) -> Vec<u8> {
    let mut out = b"GGUF".to_vec();
    out.extend(3u32.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    let kv: [(&str, Option<u32>); 5] = [
        ("general.architecture", None),
        ("llama.block_count", Some(1)),
        ("llama.attention.head_count", Some(1)),
        ("llama.attention.head_count_kv", Some(1)),
        ("llama.attention.key_length", Some(1)),
    ];
    out.extend((kv.len() as u64).to_le_bytes());
    for (k, v) in kv {
        out.extend((k.len() as u64).to_le_bytes());
        out.extend(k.as_bytes());
        match v {
            None => {
                out.extend(8u32.to_le_bytes());
                out.extend(5u64.to_le_bytes());
                out.extend(b"llama");
            }
            Some(n) => {
                out.extend(4u32.to_le_bytes());
                out.extend(n.to_le_bytes());
            }
        }
    }
    out.resize(len, 0);
    out
}

/// Starts a mock Hub; returns its base URL. `chunk_delay` slows file downloads (so progress can be
/// seen); the server lives as long as the process.
pub fn mock_hub(repos: Vec<MockRepo>, chunk_delay: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let url = format!("http://{}", listener.local_addr().expect("an address"));
    let repos = Arc::new(repos);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let repos = repos.clone();
            std::thread::spawn(move || serve_one(stream, &repos, chunk_delay));
        }
    });
    url
}

fn serve_one(mut stream: std::net::TcpStream, repos: &[MockRepo], chunk_delay: Duration) {
    let Ok(clone) = stream.try_clone() else { return };
    let mut reader = BufReader::new(clone);
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
    }
    let target = first.split_whitespace().nth(1).unwrap_or("/").to_string();
    let path = target.split('?').next().unwrap_or("").to_string();
    let query = target.split_once("search=").map(|(_, q)| q.split('&').next().unwrap_or("").to_lowercase()).unwrap_or_default();
    let json = |body: String| -> Option<Vec<u8>> { Some(body.into_bytes()) };
    let mut slow = false;
    let body: Option<Vec<u8>> = if path == "/api/models" {
        let hits: Vec<String> = repos
            .iter()
            .filter(|r| query.is_empty() || query.split(['+', ' ', '%']).filter(|t| !t.is_empty()).any(|t| r.id.to_lowercase().contains(t)))
            .map(|r| format!(r#"{{"id":"{}","downloads":{},"likes":{},"gated":false}}"#, r.id, r.downloads, r.likes))
            .collect();
        json(format!("[{}]", hits.join(",")))
    } else if let Some(r) = path.strip_prefix("/api/models/").and_then(|id| repos.iter().find(|r| r.id == id)) {
        let sib: Vec<String> = r.files.iter().map(|(n, s)| format!(r#"{{"rfilename":"{n}","size":{s}}}"#)).collect();
        json(format!(r#"{{"id":"{}","siblings":[{}]}}"#, r.id, sib.join(",")))
    } else {
        repos.iter().filter(|r| r.downloadable).find_map(|r| {
            let rest = path.strip_prefix(&format!("/{}/resolve/main/", r.id))?;
            let (_, size) = r.files.iter().find(|(n, _)| n == rest)?;
            slow = true;
            Some(gguf(*size as usize))
        })
    };
    match body {
        Some(b) => {
            let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len()).as_bytes());
            if slow && !chunk_delay.is_zero() {
                for c in b.chunks(64 * 1024) {
                    if stream.write_all(c).is_err() {
                        return;
                    }
                    let _ = stream.flush();
                    std::thread::sleep(chunk_delay);
                }
            } else {
                let _ = stream.write_all(&b);
            }
        }
        None => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        }
    }
}

/// A `ModelsService` on `dir` against `hub`, with `usable` bytes of model memory, admitting
/// whoever `auth` admits.
pub fn service(dir: &std::path::Path, hub: &str, usable: u64, auth: Arc<dyn Authorize>) -> Arc<ModelsService> {
    service_keeping(dir, hub, usable, auth, 0)
}

/// `keep_free` for a node that keeps a little more free than its disk has when it starts, so
/// every upload is refused with no-space.
pub const KEEP_MORE_THAN_FREE: u64 = u64::MAX;

/// [`service`] taking uploads that leave `keep_free` bytes free on its disk.
pub fn service_keeping(dir: &std::path::Path, hub: &str, usable: u64, auth: Arc<dyn Authorize>, keep_free: u64) -> Arc<ModelsService> {
    service_peering(dir, hub, usable, auth, keep_free, None)
}

/// [`service_keeping`] that copies uploads from other nodes over `peers` (a pool upload).
pub fn service_peering(
    dir: &std::path::Path,
    hub: &str,
    usable: u64,
    auth: Arc<dyn Authorize>,
    keep_free: u64,
    peers: Option<Arc<dyn weirdnet_agent::remote::Dialer>>,
) -> Arc<ModelsService> {
    use weirdnet_backends::models::{KeepFree, WritePolicy};
    const GIB: u64 = 1 << 30;
    let sup = Supervisor::new(vec![], vec![], SupervisorOptions::default());
    let budget = Budget { pool_bytes: usable, reserve_bytes: 0, usable_bytes: usable };
    let store = ModelStore::new(dir).with_hub(hub);
    let keep_free = if keep_free == KEEP_MORE_THAN_FREE { (store.free_bytes() / GIB + 2) * GIB } else { keep_free };
    let policy = WritePolicy::default().with_keep_free(KeepFree::new(0, keep_free));
    let svc = ModelsService::new(store, sup, auth).with_budget(budget).with_write_policy(policy);
    Arc::new(match peers {
        Some(d) => svc.with_peer_dialer(d, None),
        None => svc,
    })
}
