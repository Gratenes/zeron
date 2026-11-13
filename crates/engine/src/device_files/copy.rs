//! A copy between two devices' files: read chunks from one end, write them to
//! the other, from whichever engine was asked. An end that is this engine is
//! answered in process; any other goes over its relay ([`RemotePeer`]).
//!
//! A destination file is a `.zeron-part` beside its target until the last
//! chunk has checked the source's sha256, so a dropped link leaves a resumable
//! partial and never a half file under the real name: asking again continues
//! from the bytes the destination holds. A source that changed meanwhile fails
//! the check, and the copy restarts once from zero.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::{
    CHUNK_BYTES, MAX_CALL_BYTES, MAX_FILE_BYTES, PathParams, Policy, ReadParams, WritePartParams,
};

/// One kind of call on a device's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    List,
    Stat,
    Hash,
    Read,
    WritePart,
}

/// The reply deadline of an ordinary call, and of the first chunk of a write
/// (a phone may wait on its person's confirmation before answering it).
pub const CALL_DEADLINE: Duration = Duration::from_secs(45);
pub const FIRST_WRITE_DEADLINE: Duration = Duration::from_secs(90);
const RETRIES: usize = 3;
const BACKOFF: [Duration; RETRIES] = [
    Duration::from_millis(500),
    Duration::from_secs(2),
    Duration::from_secs(5),
];
/// Chunks read ahead of the one being written.
const READ_AHEAD: usize = 4;
/// Restarts after the destination reports a different offset than we hold.
const RESYNCS: usize = 3;

/// A device's files, as this copy sees it.
#[async_trait]
pub trait Peer: Send + Sync {
    /// The reply, or `files.<code>: message`.
    async fn call(&self, op: Op, params: Value, deadline: Duration) -> Result<Value, String>;
}

/// This engine's own files.
pub struct LocalPeer(pub Policy);

#[async_trait]
impl Peer for LocalPeer {
    async fn call(&self, op: Op, params: Value, _deadline: Duration) -> Result<Value, String> {
        let policy = self.0.clone();
        tokio::task::spawn_blocking(move || run_local(&policy, op, params))
            .await
            .map_err(|e| format!("files.unavailable: {e}"))?
    }
}

/// An op answered from `policy`, as the RPC handlers do.
pub fn run_local(policy: &Policy, op: Op, params: Value) -> Result<Value, String> {
    let bad = |e: serde_json::Error| format!("files.denied: bad parameters: {e}");
    let reply = match op {
        Op::List => super::list(
            policy,
            &serde_json::from_value::<PathParams>(params).map_err(bad)?,
        ),
        Op::Stat => super::stat(
            policy,
            &serde_json::from_value::<PathParams>(params).map_err(bad)?,
        ),
        Op::Hash => super::hash(
            policy,
            &serde_json::from_value::<PathParams>(params).map_err(bad)?,
        ),
        Op::Read => super::read(
            policy,
            &serde_json::from_value::<ReadParams>(params).map_err(bad)?,
        ),
        Op::WritePart => super::write_part(
            policy,
            &serde_json::from_value::<WritePartParams>(params).map_err(bad)?,
        ),
    };
    reply.map_err(|e| e.to_string())
}

/// What a copy of one file did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileResult {
    pub from: String,
    pub to: String,
    pub bytes: u64,
    /// `copied`, `identical` (already there, left alone) or `failed`.
    pub outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A copy's progress, as `DeviceFileCopyStatus` reports it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub transfer_id: String,
    /// `running`, `done` (every file landed or was identical) or `failed`.
    pub state: &'static str,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_total: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    pub files: Vec<FileResult>,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip)]
    started: Option<Instant>,
}

type JobTable = (HashMap<String, Arc<Mutex<Job>>>, VecDeque<String>);

/// Copies this engine has run lately.
#[derive(Clone, Default)]
pub struct Jobs {
    inner: Arc<Mutex<JobTable>>,
}

impl Jobs {
    pub fn start(&self) -> (String, Arc<Mutex<Job>>) {
        let id = uuid::Uuid::new_v4().to_string();
        let job = Arc::new(Mutex::new(Job {
            transfer_id: id.clone(),
            state: "running",
            bytes_done: 0,
            bytes_total: 0,
            files_total: 0,
            current: None,
            files: Vec::new(),
            elapsed_ms: 0,
            error: None,
            started: Some(Instant::now()),
        }));
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.0.insert(id.clone(), job.clone());
        inner.1.push_back(id.clone());
        while inner.1.len() > 50 {
            if let Some(old) = inner.1.pop_front() {
                inner.0.remove(&old);
            }
        }
        (id, job)
    }

    pub fn get(&self, id: &str) -> Option<Job> {
        let job = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0
            .get(id)
            .cloned()?;
        let mut job = job
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if job.state == "running" {
            job.elapsed_ms = job.started.map_or(0, |s| s.elapsed().as_millis() as u64);
        }
        Some(job)
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub from_path: String,
    pub to_path: String,
    pub overwrite: bool,
    pub recursive: bool,
}

fn with_job(job: &Arc<Mutex<Job>>, f: impl FnOnce(&mut Job)) {
    f(&mut job
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner));
}

fn code_of(error: &str) -> Option<&str> {
    error.strip_prefix("files.")?.split(':').next()
}

/// The bytes the destination says it holds, from a `files.bad_offset` message.
fn held_in(error: &str) -> Option<u64> {
    let rest = error.split("holds ").nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

fn join(base: &str, name: &str) -> String {
    let sep = if base.contains('\\') && !base.contains('/') {
        '\\'
    } else {
        '/'
    };
    format!("{}{}{}", base.trim_end_matches(['/', '\\']), sep, name)
}

fn file_name(path: &str) -> &str {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
}

/// Run `request`, recording progress in `job`; it ends `done` or `failed`.
pub async fn run(src: Arc<dyn Peer>, dst: Arc<dyn Peer>, request: Request, job: Arc<Mutex<Job>>) {
    let outcome = run_inner(&src, &dst, &request, &job).await;
    with_job(&job, |j| {
        j.current = None;
        match outcome {
            Ok(()) if j.files.iter().all(|f| f.outcome != "failed") => j.state = "done",
            Ok(()) => {
                j.state = "failed";
                j.error = j.files.iter().find_map(|f| f.error.clone());
            }
            Err(error) => {
                j.state = "failed";
                j.error = Some(error);
            }
        }
        j.elapsed_ms = j.started.map_or(0, |s| s.elapsed().as_millis() as u64);
    });
}

struct Item {
    from: String,
    to: String,
    size: u64,
}

async fn run_inner(
    src: &Arc<dyn Peer>,
    dst: &Arc<dyn Peer>,
    request: &Request,
    job: &Arc<Mutex<Job>>,
) -> Result<(), String> {
    let stat = call_retry(
        src,
        Op::Stat,
        json!({ "path": request.from_path }),
        CALL_DEADLINE,
    )
    .await?;
    let mut items = Vec::new();
    if stat["kind"] == "dir" {
        if !request.recursive {
            return Err(format!(
                "{} is a folder (pass recursive to copy it)",
                request.from_path
            ));
        }
        collect(src, &request.from_path, &request.to_path, &mut items).await?;
    } else {
        let to = if request.to_path.ends_with(['/', '\\']) {
            join(&request.to_path, file_name(&request.from_path))
        } else {
            request.to_path.clone()
        };
        items.push(Item {
            from: request.from_path.clone(),
            to,
            size: stat["size"].as_u64().unwrap_or(0),
        });
    }
    let total: u64 = items.iter().map(|i| i.size).sum();
    if let Some(big) = items.iter().find(|i| i.size > MAX_FILE_BYTES) {
        return Err(format!(
            "{} is {} bytes; files over {MAX_FILE_BYTES} bytes aren't moved",
            big.from, big.size
        ));
    }
    if total > MAX_CALL_BYTES {
        return Err(format!(
            "{total} bytes is over the {MAX_CALL_BYTES}-byte limit of one copy"
        ));
    }
    with_job(job, |j| {
        j.bytes_total = total;
        j.files_total = items.len();
    });
    let make_parents = request.recursive || request.to_path.ends_with(['/', '\\']);
    for item in items {
        with_job(job, |j| j.current = Some(item.from.clone()));
        let before = job
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .bytes_done;
        let result = copy_file(src, dst, &item, request.overwrite, make_parents, job).await;
        with_job(job, |j| {
            j.files.push(match result {
                Ok((outcome, sha, size)) => {
                    // Progress counts what the file really was (the hashed size), not what an
                    // earlier listing said.
                    j.bytes_done = before + size;
                    FileResult {
                        from: item.from.clone(),
                        to: item.to.clone(),
                        bytes: size,
                        outcome,
                        sha256: Some(sha),
                        error: None,
                    }
                }
                // A failed file moves progress only as far as its bytes really went.
                Err(error) => FileResult {
                    from: item.from.clone(),
                    to: item.to.clone(),
                    bytes: 0,
                    outcome: "failed",
                    sha256: None,
                    error: Some(error),
                },
            });
        });
    }
    Ok(())
}

async fn collect(
    src: &Arc<dyn Peer>,
    dir: &str,
    to: &str,
    out: &mut Vec<Item>,
) -> Result<(), String> {
    let mut stack = vec![(dir.to_owned(), to.to_owned())];
    while let Some((dir, to)) = stack.pop() {
        let mut cursor: Option<String> = None;
        loop {
            let reply = call_retry(
                src,
                Op::List,
                {
                    // No `cursor` on the first page: a phone reads a JSON null as the text "null".
                    let mut params = json!({ "path": dir });
                    if let Some(c) = &cursor {
                        params["cursor"] = json!(c);
                    }
                    params
                },
                CALL_DEADLINE,
            )
            .await?;
            for e in reply["entries"].as_array().cloned().unwrap_or_default() {
                let name = e["name"].as_str().unwrap_or_default();
                let path = e["path"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| join(&dir, name));
                if e["kind"] == "dir" {
                    stack.push((path, join(&to, name)));
                } else {
                    out.push(Item {
                        from: path,
                        to: join(&to, name),
                        size: e["size"].as_u64().unwrap_or(0),
                    });
                }
                if out.len() > 100_000 {
                    return Err("more than 100000 files; copy a smaller folder".into());
                }
            }
            match reply["cursor"].as_str() {
                Some(c) => cursor = Some(c.to_owned()),
                None => break,
            }
        }
    }
    out.sort_by(|a, b| a.from.cmp(&b.from));
    Ok(())
}

/// A call, tried again (with backoff) when only the transport failed.
async fn call_retry(
    peer: &Arc<dyn Peer>,
    op: Op,
    params: Value,
    deadline: Duration,
) -> Result<Value, String> {
    let mut last = String::new();
    for attempt in 0..=RETRIES {
        if attempt > 0 {
            tokio::time::sleep(BACKOFF[attempt - 1]).await;
        }
        match tokio::time::timeout(deadline, peer.call(op, params.clone(), deadline)).await {
            Ok(Ok(value)) => return Ok(value),
            // A refusal from the device is final; only a dropped or silent link is retried.
            Ok(Err(error)) if code_of(&error).is_some_and(|c| c != "unavailable") => {
                return Err(error);
            }
            Ok(Err(error)) => last = error,
            Err(_) => {
                last = format!(
                    "files.unavailable: no answer within {}s",
                    deadline.as_secs()
                )
            }
        }
    }
    Err(last)
}

/// One file. `Ok((outcome, sha256))`.
async fn copy_file(
    src: &Arc<dyn Peer>,
    dst: &Arc<dyn Peer>,
    item: &Item,
    overwrite: bool,
    make_parents: bool,
    job: &Arc<Mutex<Job>>,
) -> Result<(&'static str, String, u64), String> {
    let hashed = call_retry(
        src,
        Op::Hash,
        json!({ "path": item.from }),
        Duration::from_secs(600),
    )
    .await?;
    let mut sha = hashed["sha256"]
        .as_str()
        .ok_or("the source gave no sha256")?
        .to_owned();
    let size = hashed["size"].as_u64().unwrap_or(item.size);
    match call_retry(dst, Op::Stat, json!({ "path": item.to }), CALL_DEADLINE).await {
        Ok(existing) if existing["kind"] == "file" => {
            if existing["size"].as_u64() == Some(size) {
                let there = call_retry(
                    dst,
                    Op::Hash,
                    json!({ "path": item.to }),
                    Duration::from_secs(600),
                )
                .await?;
                if there["sha256"].as_str() == Some(sha.as_str()) {
                    return Ok(("identical", sha, size));
                }
            }
            if !overwrite {
                return Err(format!(
                    "files.exists: {} exists (pass overwrite to replace it)",
                    item.to
                ));
            }
        }
        Ok(_) => return Err(format!("files.exists: {} is a folder", item.to)),
        Err(error) if code_of(&error) == Some("not_found") => {}
        Err(error) => return Err(error),
    }
    // Resume: a partial the destination already holds (its part file sits beside the target).
    let mut offset = match call_retry(
        dst,
        Op::Stat,
        json!({ "path": format!("{}.zeron-part", item.to) }),
        CALL_DEADLINE,
    )
    .await
    {
        Ok(part) => part["size"]
            .as_u64()
            .filter(|held| *held <= size)
            .unwrap_or(0),
        Err(_) => 0,
    };
    let base = job
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .bytes_done;
    let mut restarted = false;
    let mut resyncs = 0;
    loop {
        match stream(
            src,
            dst,
            item,
            size,
            &sha,
            offset,
            overwrite,
            make_parents,
            job,
            base,
        )
        .await
        {
            Ok(()) => return Ok(("copied", sha, size)),
            Err(Stop::Resync(held)) if resyncs < RESYNCS => {
                resyncs += 1;
                offset = held.min(size);
            }
            // What arrived hashed differently: the resumed partial or the source was not
            // what it was. The destination dropped the partial; once from zero.
            Err(Stop::Failed(error))
                if !restarted && offset > 0 && code_of(&error) == Some("bad_hash") =>
            {
                restarted = true;
                offset = 0;
                let again = call_retry(
                    src,
                    Op::Hash,
                    json!({ "path": item.from }),
                    Duration::from_secs(600),
                )
                .await?;
                sha = again["sha256"]
                    .as_str()
                    .ok_or("the source gave no sha256")?
                    .to_owned();
            }
            Err(Stop::Resync(held)) => {
                return Err(format!(
                    "files.bad_offset: the destination keeps reporting {held} bytes held"
                ));
            }
            Err(Stop::Failed(error)) => return Err(error),
        }
    }
}

enum Stop {
    /// The destination holds this many bytes: continue from there.
    Resync(u64),
    Failed(String),
}

#[allow(clippy::too_many_arguments)]
async fn stream(
    src: &Arc<dyn Peer>,
    dst: &Arc<dyn Peer>,
    item: &Item,
    size: u64,
    sha: &str,
    start: u64,
    overwrite: bool,
    make_parents: bool,
    job: &Arc<Mutex<Job>>,
    base: u64,
) -> Result<(), Stop> {
    let (tx, mut rx) = mpsc::channel::<Result<(u64, Vec<u8>), String>>(READ_AHEAD);
    let reader = {
        let src = src.clone();
        let from = item.from.clone();
        tokio::spawn(async move {
            let mut offset = start;
            loop {
                let bytes = if offset >= size {
                    Vec::new()
                } else {
                    let reply = call_retry(
                        &src,
                        Op::Read,
                        json!({ "path": from, "offset": offset, "len": CHUNK_BYTES }),
                        CALL_DEADLINE,
                    )
                    .await;
                    match reply {
                        Ok(reply) if reply["size"].as_u64() != Some(size) => {
                            let _ = tx
                                .send(Err(
                                    "files.bad_hash: the source changed size while it was copied"
                                        .into(),
                                ))
                                .await;
                            return;
                        }
                        Ok(reply) => match B64.decode(reply["data"].as_str().unwrap_or_default()) {
                            Ok(bytes) if !bytes.is_empty() => bytes,
                            Ok(_) => {
                                let _ = tx.send(Err("files.unavailable: the source returned no bytes before its end".into())).await;
                                return;
                            }
                            Err(e) => {
                                let _ = tx
                                    .send(Err(format!(
                                        "files.unavailable: bad data from the source: {e}"
                                    )))
                                    .await;
                                return;
                            }
                        },
                        Err(error) => {
                            let _ = tx.send(Err(error)).await;
                            return;
                        }
                    }
                };
                let next = offset + bytes.len() as u64;
                if tx.send(Ok((offset, bytes))).await.is_err() {
                    return;
                }
                offset = next;
                if offset >= size {
                    return;
                }
            }
        })
    };
    let mut first = true;
    let result = loop {
        let Some(chunk) = rx.recv().await else {
            break Err(Stop::Failed(
                "files.unavailable: the source stopped early".into(),
            ));
        };
        let (offset, bytes) = match chunk {
            Ok(c) => c,
            Err(e) => break Err(Stop::Failed(e)),
        };
        let end = offset + bytes.len() as u64;
        let last = end >= size;
        let mut params = json!({
            "transferId": job.lock().unwrap_or_else(std::sync::PoisonError::into_inner).transfer_id.clone(),
            "path": item.to,
            "offset": offset,
            "data": B64.encode(&bytes),
            "final": last,
            "overwrite": overwrite,
        });
        if last {
            params["sha256"] = json!(sha);
        }
        if make_parents {
            params["makeParents"] = json!(true);
        }
        let deadline = if first {
            FIRST_WRITE_DEADLINE
        } else {
            CALL_DEADLINE
        };
        first = false;
        match call_retry(dst, Op::WritePart, params, deadline).await {
            Ok(_) => with_job(job, |j| j.bytes_done = base + end),
            Err(error) if code_of(&error) == Some("bad_offset") => match held_in(&error) {
                Some(held) => break Err(Stop::Resync(held)),
                None => break Err(Stop::Failed(error)),
            },
            Err(error) => break Err(Stop::Failed(error)),
        }
        if last {
            break Ok(());
        }
    };
    reader.abort();
    result
}

/// An engine's other devices' files, over their relay.
pub struct RemotePeer {
    pub links: Arc<zeron_rpc::LinkCache>,
    pub target: String,
}

fn method_of(op: Op) -> &'static str {
    use zeron_rpc::methods;
    match op {
        Op::List => methods::DEVICE_FILE_LIST,
        Op::Stat => methods::DEVICE_FILE_STAT,
        Op::Hash => methods::DEVICE_FILE_HASH,
        Op::Read => methods::DEVICE_FILE_READ,
        Op::WritePart => methods::DEVICE_FILE_WRITE_PART,
    }
}

#[async_trait]
impl Peer for RemotePeer {
    async fn call(&self, op: Op, params: Value, _deadline: Duration) -> Result<Value, String> {
        let client = self
            .links
            .client(&self.target)
            .await
            .map_err(|e| format!("files.unavailable: {e}"))?;
        match client.call(method_of(op), params).await {
            Ok(value) => Ok(value),
            Err(error) => {
                if matches!(error, zeron_rpc::RpcError::Closed) {
                    self.links.invalidate(&self.target);
                }
                // The remote's own `files.*` refusals pass through untouched.
                let text = error.to_string();
                Err(match text.find("files.") {
                    Some(at) => text[at..].to_owned(),
                    None => format!("files.unavailable: {text}"),
                })
            }
        }
    }
}

/// A phone's files: Zeron for Android answers `files_*` tools over `PhoneCall`, and decides for
/// itself which folders it lets an agent reach.
pub struct PhonePeer {
    pub links: Arc<zeron_rpc::LinkCache>,
    pub device: zeron_proto::Device,
}

fn tool_of(op: Op) -> &'static str {
    match op {
        Op::List => "files_list",
        Op::Stat => "files_stat",
        Op::Hash => "files_hash",
        Op::Read => "files_read",
        Op::WritePart => "files_write_part",
    }
}

#[async_trait]
impl Peer for PhonePeer {
    async fn call(&self, op: Op, mut params: Value, deadline: Duration) -> Result<Value, String> {
        if let Some(args) = params.as_object_mut() {
            // Not the phone's to know: this engine made the missing folders itself.
            args.remove("makeParents");
            // Android reads a JSON null as the text "null" (`optString`): leave absent things out.
            args.retain(|_, v| !v.is_null());
            args.remove("targetDeviceId");
        }
        let call = json!({ "targetDeviceId": self.device.id, "tool": tool_of(op), "args": params });
        let reply =
            crate::rpc::EngineRpc::ask_line(self.links.clone(), &self.device, call, deadline).await;
        match reply {
            Ok(zeron_rpc::RpcReply::Value(value)) => Ok(value),
            Ok(_) => Err("files.unavailable: the phone answered with a stream".into()),
            Err(error) => {
                let text = error.to_string();
                Err(match text.find("files.") {
                    Some(at) => text[at..].to_owned(),
                    None => format!("files.unavailable: {text}"),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Side {
        _dir: tempfile::TempDir,
        root: std::path::PathBuf,
        peer: Arc<dyn Peer>,
    }

    fn side() -> Side {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let data = root.join(".zeron-data");
        fs::create_dir_all(&data).unwrap();
        let peer: Arc<dyn Peer> = Arc::new(LocalPeer(Policy::new(&root, &data, [])));
        Side {
            _dir: dir,
            root,
            peer,
        }
    }

    fn req(from: &std::path::Path, to: &std::path::Path) -> Request {
        Request {
            from_path: from.display().to_string(),
            to_path: to.display().to_string(),
            overwrite: false,
            recursive: false,
        }
    }

    async fn copy(a: &Side, b: &Side, request: Request) -> Job {
        let jobs = Jobs::default();
        let (id, job) = jobs.start();
        run(a.peer.clone(), b.peer.clone(), request, job).await;
        jobs.get(&id).unwrap()
    }

    fn pattern(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 % 251) as u8).collect()
    }

    /// Fails the next `n` calls of `op` as a dropped link, then passes through.
    struct Flaky {
        inner: Arc<dyn Peer>,
        op: Op,
        after: AtomicUsize,
        fail: AtomicUsize,
        seen: Mutex<Vec<(Op, Value)>>,
    }

    #[async_trait]
    impl Peer for Flaky {
        async fn call(&self, op: Op, params: Value, deadline: Duration) -> Result<Value, String> {
            self.seen.lock().unwrap().push((op, params.clone()));
            if op == self.op {
                if self.after.load(Ordering::SeqCst) > 0 {
                    self.after.fetch_sub(1, Ordering::SeqCst);
                } else if self.fail.load(Ordering::SeqCst) > 0 {
                    self.fail.fetch_sub(1, Ordering::SeqCst);
                    return Err("files.unavailable: link dropped".into());
                }
            }
            self.inner.call(op, params, deadline).await
        }
    }

    #[tokio::test]
    async fn copies_a_multi_chunk_file_with_a_matching_hash() {
        let (a, b) = (side(), side());
        let data = pattern(1_700_000);
        fs::write(a.root.join("rec.m4a"), &data).unwrap();
        let job = copy(
            &a,
            &b,
            req(&a.root.join("rec.m4a"), &b.root.join("rec.m4a")),
        )
        .await;
        assert_eq!(job.state, "done", "{:?}", job.error);
        assert_eq!(job.files[0].outcome, "copied");
        assert_eq!(job.bytes_done, 1_700_000);
        assert_eq!(fs::read(b.root.join("rec.m4a")).unwrap(), data);
        assert!(!b.root.join("rec.m4a.zeron-part").exists());
    }

    #[tokio::test]
    async fn an_empty_file_copies() {
        let (a, b) = (side(), side());
        fs::write(a.root.join("e"), b"").unwrap();
        let job = copy(&a, &b, req(&a.root.join("e"), &b.root.join("e"))).await;
        assert_eq!(job.state, "done", "{:?}", job.error);
        assert_eq!(fs::read(b.root.join("e")).unwrap().len(), 0);
    }

    #[tokio::test]
    async fn an_identical_file_is_left_alone_and_a_different_one_needs_overwrite() {
        let (a, b) = (side(), side());
        fs::write(a.root.join("f"), b"same").unwrap();
        fs::write(b.root.join("f"), b"same").unwrap();
        let job = copy(&a, &b, req(&a.root.join("f"), &b.root.join("f"))).await;
        assert_eq!((job.state, job.files[0].outcome), ("done", "identical"));
        fs::write(b.root.join("f"), b"diff").unwrap();
        let job = copy(&a, &b, req(&a.root.join("f"), &b.root.join("f"))).await;
        assert_eq!(job.state, "failed");
        assert!(job.error.unwrap().starts_with("files.exists"));
        assert_eq!(fs::read(b.root.join("f")).unwrap(), b"diff");
        let mut r = req(&a.root.join("f"), &b.root.join("f"));
        r.overwrite = true;
        assert_eq!(copy(&a, &b, r).await.state, "done");
        assert_eq!(fs::read(b.root.join("f")).unwrap(), b"same");
    }

    #[tokio::test]
    async fn a_dropped_link_is_retried_and_a_dead_one_leaves_a_partial_that_resumes() {
        let (a, b) = (side(), side());
        let data = pattern(1_700_000);
        fs::write(a.root.join("big"), &data).unwrap();
        // One dropped write: retried (after the first backoff), copy completes.
        let flaky = Arc::new(Flaky {
            inner: b.peer.clone(),
            op: Op::WritePart,
            after: AtomicUsize::new(1),
            fail: AtomicUsize::new(1),
            seen: Mutex::new(vec![]),
        });
        let jobs = Jobs::default();
        let (_, job) = jobs.start();
        run(
            a.peer.clone(),
            flaky,
            req(&a.root.join("big"), &b.root.join("big")),
            job.clone(),
        )
        .await;
        assert_eq!(job.lock().unwrap().state, "done");
        assert_eq!(fs::read(b.root.join("big")).unwrap(), data);

        // A link that stays down after two chunks: fails, partial kept…
        fs::remove_file(b.root.join("big")).unwrap();
        let dead = Arc::new(Flaky {
            inner: b.peer.clone(),
            op: Op::WritePart,
            after: AtomicUsize::new(2),
            fail: AtomicUsize::new(100),
            seen: Mutex::new(vec![]),
        });
        let (_, job) = jobs.start();
        tokio::time::pause();
        run(
            a.peer.clone(),
            dead,
            req(&a.root.join("big"), &b.root.join("big")),
            job.clone(),
        )
        .await;
        tokio::time::resume();
        assert_eq!(job.lock().unwrap().state, "failed");
        let held = fs::metadata(b.root.join("big.zeron-part")).unwrap().len();
        assert_eq!(held, 2 * CHUNK_BYTES);
        // …and asking again continues from the held bytes (no write below them).
        let watch = Arc::new(Flaky {
            inner: b.peer.clone(),
            op: Op::List,
            after: AtomicUsize::new(0),
            fail: AtomicUsize::new(0),
            seen: Mutex::new(vec![]),
        });
        let (_, job) = jobs.start();
        run(
            a.peer.clone(),
            watch.clone(),
            req(&a.root.join("big"), &b.root.join("big")),
            job.clone(),
        )
        .await;
        assert_eq!(job.lock().unwrap().state, "done");
        let first_offset = watch
            .seen
            .lock()
            .unwrap()
            .iter()
            .find(|(op, _)| *op == Op::WritePart)
            .map(|(_, p)| p["offset"].as_u64().unwrap())
            .unwrap();
        assert_eq!(first_offset, held);
        assert_eq!(fs::read(b.root.join("big")).unwrap(), data);
    }

    #[tokio::test]
    async fn a_stale_partial_is_caught_by_the_hash_and_the_copy_restarts_from_zero() {
        let (a, b) = (side(), side());
        let data = pattern(1_200_000);
        fs::write(a.root.join("s"), &data).unwrap();
        // A partial from an earlier, different source.
        fs::write(b.root.join("s.zeron-part"), vec![9u8; 600_000]).unwrap();
        let job = copy(&a, &b, req(&a.root.join("s"), &b.root.join("s"))).await;
        assert_eq!(job.state, "done", "{:?}", job.error);
        assert_eq!(fs::read(b.root.join("s")).unwrap(), data);
        assert!(!b.root.join("s.zeron-part").exists());
    }

    #[tokio::test]
    async fn a_refused_path_fails_without_retrying_and_a_folder_needs_recursive() {
        let (a, b) = (side(), side());
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("x"), b"x").unwrap();
        let job = copy(&a, &b, req(&outside.path().join("x"), &b.root.join("x"))).await;
        assert_eq!(job.state, "failed");
        assert!(job.error.unwrap().starts_with("files.no_grant"));
        fs::create_dir(a.root.join("d")).unwrap();
        let job = copy(&a, &b, req(&a.root.join("d"), &b.root.join("d"))).await;
        assert!(job.error.unwrap().contains("recursive"));
    }

    #[tokio::test]
    async fn a_folder_copies_recursively_with_its_subfolders() {
        let (a, b) = (side(), side());
        fs::create_dir_all(a.root.join("calls/2026")).unwrap();
        fs::write(a.root.join("calls/a.m4a"), pattern(10)).unwrap();
        fs::write(a.root.join("calls/2026/b.m4a"), pattern(700_000)).unwrap();
        let mut r = req(&a.root.join("calls"), &b.root.join("calls"));
        r.recursive = true;
        let job = copy(&a, &b, r).await;
        assert_eq!(job.state, "done", "{:?}", job.error);
        assert_eq!(job.files_total, 2);
        assert_eq!(job.bytes_done, job.bytes_total);
        assert_eq!(
            fs::read(b.root.join("calls/2026/b.m4a")).unwrap(),
            pattern(700_000)
        );
    }

    #[tokio::test]
    async fn a_file_bigger_than_the_cap_is_refused_before_any_byte_moves() {
        let (a, b) = (side(), side());
        let f = fs::File::create(a.root.join("huge")).unwrap();
        f.set_len(MAX_FILE_BYTES + 1).unwrap();
        let job = copy(&a, &b, req(&a.root.join("huge"), &b.root.join("huge"))).await;
        assert_eq!(job.state, "failed");
        assert!(job.error.unwrap().contains("aren't moved"));
        assert!(!b.root.join("huge.zeron-part").exists());
    }

    /// A phone as `optString` reads it: a null `cursor` is the text "null", so every name that
    /// sorts before it is filtered out of the page.
    struct NullIsText(Arc<dyn Peer>);

    #[async_trait]
    impl Peer for NullIsText {
        async fn call(&self, op: Op, params: Value, deadline: Duration) -> Result<Value, String> {
            if op == Op::List && params.get("cursor").is_some_and(Value::is_null) {
                return Ok(json!({ "entries": [] }));
            }
            self.0.call(op, params, deadline).await
        }
    }

    #[tokio::test]
    async fn a_folder_on_a_source_that_reads_a_null_cursor_as_text_still_lists() {
        let (a, b) = (side(), side());
        fs::create_dir_all(a.root.join("Shiz")).unwrap();
        for name in ["2026-01-01 Call.ogg", "Alice.ogg", "call.m4a"] {
            fs::write(a.root.join("Shiz").join(name), pattern(2000)).unwrap();
        }
        let src = Side {
            _dir: tempfile::tempdir().unwrap(),
            root: a.root.clone(),
            peer: Arc::new(NullIsText(a.peer.clone())),
        };
        let mut r = req(&a.root.join("Shiz"), &b.root.join("Call Recordings/"));
        r.recursive = true;
        let job = copy(&src, &b, r).await;
        assert_eq!(job.state, "done", "{:?}", job.error);
        assert_eq!(job.files_total, 3);
        assert_eq!(
            fs::read(b.root.join("Call Recordings/Alice.ogg")).unwrap(),
            pattern(2000)
        );
    }

    #[tokio::test]
    async fn a_single_file_into_a_folder_that_does_not_exist_yet_makes_it() {
        let (a, b) = (side(), side());
        fs::write(a.root.join("x.ogg"), pattern(100)).unwrap();
        let job = copy(
            &a,
            &b,
            Request {
                from_path: a.root.join("x.ogg").display().to_string(),
                to_path: format!("{}/Call Recordings/", b.root.display()),
                overwrite: false,
                recursive: false,
            },
        )
        .await;
        assert_eq!(job.state, "done", "{:?}", job.error);
        assert_eq!(
            fs::read(b.root.join("Call Recordings/x.ogg")).unwrap(),
            pattern(100)
        );
    }

    #[tokio::test]
    async fn a_failed_file_does_not_count_as_moved() {
        let (a, b) = (side(), side());
        fs::write(a.root.join("f"), b"new!").unwrap();
        fs::write(b.root.join("f"), b"old!").unwrap();
        let job = copy(&a, &b, req(&a.root.join("f"), &b.root.join("f"))).await;
        assert_eq!(
            (job.state, job.bytes_total, job.bytes_done),
            ("failed", 4, 0)
        );
        assert_eq!(job.files[0].bytes, 0);
    }
}
