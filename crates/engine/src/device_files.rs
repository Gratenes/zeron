//! Device files: this device's files, offered to the account's other devices
//! (and their agents) by path, in bounded chunks. The wire shape is shared with
//! the phone's own host (`files_list`, `files_stat`, `files_hash`, `files_read`,
//! `files_write_part`), so a copy is the same loop whichever end is a phone.
//!
//! What may be touched is [`Policy`]: the home folder plus project folders (and
//! `ZERON_DEVICE_FILES_ROOTS`), minus a deny-list that always wins: credentials,
//! keys, browser profiles, keychains and this engine's own data. Paths resolve
//! through symlinks first, so a link cannot lead out. A write goes to
//! `<name>.zeron-part` beside its target and is renamed into place by a final
//! chunk that checks the size and sha256; nothing is overwritten unless asked.
//!
//! Errors read `files.<code>: message` (not_found, denied, no_grant, exists,
//! too_large, bad_offset, unavailable), as the phone's do.

use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub mod copy;

/// Raw bytes per read or write chunk (~680 KB of base64, under the relay's frame ceiling).
pub const CHUNK_BYTES: u64 = 512 * 1024;
/// Entries per list page.
pub const PAGE_ENTRIES: usize = 500;
/// The most one file may be.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// The most one copy call may move.
pub const MAX_CALL_BYTES: u64 = 20 * 1024 * 1024 * 1024;
const PART_SUFFIX: &str = ".zeron-part";

#[derive(Debug)]
pub struct FileError {
    pub code: &'static str,
    pub message: String,
}

impl FileError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    fn io(path: &Path, error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => {
                Self::new("not_found", format!("{} doesn't exist", path.display()))
            }
            std::io::ErrorKind::PermissionDenied => Self::new(
                "denied",
                format!("{} can't be accessed: {error}", path.display()),
            ),
            _ => Self::new("unavailable", format!("{}: {error}", path.display())),
        }
    }
}

impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "files.{}: {}", self.code, self.message)
    }
}

impl From<FileError> for zeron_rpc::RpcError {
    fn from(error: FileError) -> Self {
        zeron_rpc::RpcError::Failed(error.to_string())
    }
}

type Result<T> = std::result::Result<T, FileError>;

/// What this device lets other devices reach.
#[derive(Debug, Clone)]
pub struct Policy {
    roots: Vec<PathBuf>,
    /// Folders and files that are never reachable (canonical, absolute).
    deny: Vec<PathBuf>,
    home: PathBuf,
}

/// Paths under the home folder that are never reachable, `/`-separated, compared
/// case-insensitively.
const DENIED_UNDER_HOME: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker/config.json",
    ".netrc",
    "_netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    ".config/gh",
    ".config/git/credentials",
    ".config/gcloud",
    ".password-store",
    ".local/share/keyrings",
    ".claude/.credentials.json",
    ".codex/auth.json",
    ".mozilla",
    ".config/google-chrome",
    ".config/chromium",
    ".config/microsoft-edge",
    ".config/BraveSoftware",
    ".config/vivaldi",
    "snap/firefox",
    "snap/chromium",
    "Library/Keychains",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/Firefox",
    "Library/Application Support/BraveSoftware",
    "Library/Application Support/Microsoft Edge",
    "AppData/Local/Google/Chrome",
    "AppData/Local/Microsoft/Edge",
    "AppData/Local/BraveSoftware",
    "AppData/Local/Microsoft/Credentials",
    "AppData/Roaming/Mozilla",
    "AppData/Roaming/Microsoft/Credentials",
    "AppData/Roaming/Microsoft/Protect",
    "AppData/Roaming/Microsoft/Crypto",
    "AppData/Roaming/gnupg",
];

impl Policy {
    /// The home folder and `extra_roots` (project folders), with this engine's
    /// `data_dir` and the fixed deny-list excluded.
    pub fn new(
        home: &Path,
        data_dir: &Path,
        extra_roots: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        let canon = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let home = canon(home);
        let mut roots = vec![home.clone()];
        for root in extra_roots {
            let root = canon(&root);
            if root.is_absolute() && !roots.contains(&root) {
                roots.push(root);
            }
        }
        if let Some(extra) = std::env::var_os("ZERON_DEVICE_FILES_ROOTS") {
            for root in std::env::split_paths(&extra).filter(|p| p.is_absolute()) {
                let root = canon(&root);
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
        }
        let mut deny: Vec<PathBuf> = DENIED_UNDER_HOME
            .iter()
            .map(|rel| rel.split('/').fold(home.clone(), |p, part| p.join(part)))
            .map(|p| canon(&p))
            .collect();
        deny.push(canon(data_dir));
        Self { roots, deny, home }
    }

    fn denied(&self, path: &Path) -> bool {
        let lower = |p: &Path| p.to_string_lossy().to_lowercase();
        let candidate = lower(path);
        let under = |root: &str| {
            candidate == root
                || candidate
                    .strip_prefix(root)
                    .is_some_and(|rest| rest.starts_with(['/', '\\']))
        };
        if self.deny.iter().any(|d| under(&lower(d))) {
            return true;
        }
        // A repository's own git config can hold a token in a remote URL.
        let mut tail = path.components().rev();
        tail.next().is_some_and(|last| last.as_os_str() == "config")
            && tail
                .next()
                .is_some_and(|parent| parent.as_os_str() == ".git")
    }

    fn inside_roots(&self, path: &Path) -> bool {
        self.roots.iter().any(|root| path.starts_with(root))
    }

    /// An existing path, resolved through links and checked against the policy.
    fn resolve(&self, raw: &str) -> Result<PathBuf> {
        let path = self.absolute(raw)?;
        let real = fs::canonicalize(&path).map_err(|e| {
            // What lies outside the allowed folders is never said to exist or not.
            if !self.inside_roots(&path) {
                return FileError::new(
                    "no_grant",
                    format!("{raw} is outside the folders agents may reach on this device"),
                );
            }
            FileError::io(&path, e)
        })?;
        self.admit(&real, raw)?;
        Ok(real)
    }

    /// A path to be created: its parent must exist and be admitted.
    fn resolve_new(&self, raw: &str) -> Result<PathBuf> {
        let path = self.absolute(raw)?;
        let name = path
            .file_name()
            .filter(|n| !n.is_empty())
            .ok_or_else(|| FileError::new("denied", format!("{raw} has no file name")))?
            .to_owned();
        let parent = path
            .parent()
            .ok_or_else(|| FileError::new("denied", format!("{raw} has no folder")))?;
        let parent = fs::canonicalize(parent).map_err(|e| FileError::io(parent, e))?;
        let full = parent.join(name);
        self.admit(&full, raw)?;
        // A link at the target would be followed by the write.
        if let Ok(meta) = fs::symlink_metadata(&full)
            && meta.file_type().is_symlink()
        {
            let real = fs::canonicalize(&full).map_err(|e| FileError::io(&full, e))?;
            self.admit(&real, raw)?;
            return Ok(real);
        }
        Ok(full)
    }

    /// The target's missing folders, created once the nearest existing one is admitted.
    fn make_parents(&self, raw: &str) -> Result<()> {
        let path = self.absolute(raw)?;
        let mut ancestor = path.parent();
        while let Some(a) = ancestor {
            if a.exists() {
                let real = fs::canonicalize(a).map_err(|e| FileError::io(a, e))?;
                self.admit(&real, raw)?;
                break;
            }
            ancestor = a.parent();
        }
        let parent = path
            .parent()
            .ok_or_else(|| FileError::new("denied", format!("{raw} has no folder")))?;
        // Admitted ancestor + lexical `..`-free remainder: nothing created can leave it.
        fs::create_dir_all(parent).map_err(|e| FileError::io(parent, e))
    }

    fn absolute(&self, raw: &str) -> Result<PathBuf> {
        let path = if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
            self.home.join(rest)
        } else if raw == "~" {
            self.home.clone()
        } else {
            PathBuf::from(raw)
        };
        if !path.is_absolute() {
            return Err(FileError::new(
                "denied",
                format!("{raw} isn't an absolute path"),
            ));
        }
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(FileError::new(
                "no_grant",
                format!("{raw} leaves the folders agents may reach"),
            ));
        }
        Ok(path)
    }

    fn admit(&self, real: &Path, raw: &str) -> Result<()> {
        if !self.inside_roots(real) {
            return Err(FileError::new(
                "no_grant",
                format!("{raw} is outside the folders agents may reach on this device"),
            ));
        }
        if self.denied(real) {
            return Err(FileError::new(
                "denied",
                format!("{raw} is protected (credentials, keys, browser or Zeron data)"),
            ));
        }
        Ok(())
    }

    /// The reachable roots, for a listing of "".
    fn root_entries(&self) -> Vec<Value> {
        self.roots
            .iter()
            .map(|r| {
                json!({
                    "name": r.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| r.display().to_string()),
                    "path": r.display().to_string(),
                    "kind": "dir",
                    "writable": true,
                })
            })
            .collect()
    }
}

fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

fn entry(name: &str, path: &Path, meta: &fs::Metadata) -> Value {
    json!({
        "name": name,
        "path": path.display().to_string(),
        "kind": if meta.is_dir() { "dir" } else { "file" },
        "size": if meta.is_dir() { 0 } else { meta.len() },
        "mtime": mtime_ms(meta),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PathParams {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadParams {
    pub path: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub len: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WritePartParams {
    pub transfer_id: String,
    pub path: String,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub data: String,
    #[serde(default, rename = "final")]
    pub is_final: bool,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
    /// Create the target's missing folders (inside the policy) first.
    #[serde(default)]
    pub make_parents: bool,
}

pub fn list(policy: &Policy, p: &PathParams) -> Result<Value> {
    if p.path.is_empty() || p.path == "/" && policy.roots.iter().all(|r| r != Path::new("/")) {
        return Ok(json!({ "entries": policy.root_entries() }));
    }
    let dir = policy.resolve(&p.path)?;
    if !dir.is_dir() {
        return Err(FileError::new(
            "denied",
            format!("{} isn't a folder", p.path),
        ));
    }
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map_err(|e| FileError::io(&dir, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.ends_with(PART_SUFFIX))
        .collect();
    names.sort();
    let mut entries = Vec::new();
    let mut last = None;
    for name in names
        .iter()
        .filter(|n| p.cursor.as_ref().is_none_or(|c| n.as_str() > c.as_str()))
    {
        let child = dir.join(name);
        // Entries that resolve outside the policy are left out, not errors.
        let Ok(real) = fs::canonicalize(&child) else {
            continue;
        };
        if !policy.inside_roots(&real) || policy.denied(&real) {
            continue;
        }
        let Ok(meta) = fs::metadata(&real) else {
            continue;
        };
        entries.push(entry(name, &child, &meta));
        last = Some(name.clone());
        if entries.len() == PAGE_ENTRIES {
            break;
        }
    }
    let more = entries.len() == PAGE_ENTRIES
        && last
            .as_ref()
            .is_some_and(|l| names.last().is_some_and(|n| n > l));
    let mut reply = json!({ "entries": entries });
    if more {
        reply["cursor"] = json!(last);
    }
    Ok(reply)
}

pub fn stat(policy: &Policy, p: &PathParams) -> Result<Value> {
    let real = policy.resolve(&p.path)?;
    let meta = fs::metadata(&real).map_err(|e| FileError::io(&real, e))?;
    let name = real
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(entry(&name, &real, &meta))
}

fn sha256_of(path: &Path) -> Result<(String, u64)> {
    let mut file = fs::File::open(path).map_err(|e| FileError::io(path, e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(|e| FileError::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((hex(&hasher.finalize()), total))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash(policy: &Policy, p: &PathParams) -> Result<Value> {
    let real = policy.resolve(&p.path)?;
    if !real.is_file() {
        return Err(FileError::new("denied", format!("{} isn't a file", p.path)));
    }
    let (sha, size) = sha256_of(&real)?;
    Ok(json!({ "sha256": sha, "size": size, "path": real.display().to_string() }))
}

pub fn read(policy: &Policy, p: &ReadParams) -> Result<Value> {
    let len = p.len.unwrap_or(CHUNK_BYTES);
    if len > CHUNK_BYTES {
        return Err(FileError::new(
            "too_large",
            format!("len {len} is over {CHUNK_BYTES} bytes"),
        ));
    }
    let real = policy.resolve(&p.path)?;
    let meta = fs::metadata(&real).map_err(|e| FileError::io(&real, e))?;
    if !meta.is_file() {
        return Err(FileError::new("denied", format!("{} isn't a file", p.path)));
    }
    let size = meta.len();
    let mut file = fs::File::open(&real).map_err(|e| FileError::io(&real, e))?;
    file.seek(SeekFrom::Start(p.offset.min(size)))
        .map_err(|e| FileError::io(&real, e))?;
    let mut buf = vec![0u8; len.min(size.saturating_sub(p.offset)) as usize];
    file.read_exact(&mut buf)
        .map_err(|e| FileError::io(&real, e))?;
    let next = p.offset.min(size) + buf.len() as u64;
    Ok(json!({
        "data": B64.encode(&buf),
        "size": size,
        "mtime": mtime_ms(&meta),
        "nextOffset": next,
        "done": next >= size,
    }))
}

fn part_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(PART_SUFFIX);
    target.with_file_name(name)
}

pub fn write_part(policy: &Policy, p: &WritePartParams) -> Result<Value> {
    if p.transfer_id.is_empty() {
        return Err(FileError::new("denied", "a transfer needs an id"));
    }
    let bytes = B64
        .decode(p.data.as_bytes())
        .map_err(|e| FileError::new("denied", format!("data isn't base64: {e}")))?;
    if bytes.len() as u64 > CHUNK_BYTES {
        return Err(FileError::new(
            "too_large",
            format!("a chunk is at most {CHUNK_BYTES} bytes"),
        ));
    }
    if p.make_parents && p.offset == 0 {
        policy.make_parents(&p.path)?;
    }
    let target = policy.resolve_new(&p.path)?;
    if target.is_dir() {
        return Err(FileError::new("exists", format!("{} is a folder", p.path)));
    }
    if target.exists() && !p.overwrite {
        return Err(FileError::new(
            "exists",
            format!("{} exists (pass overwrite to replace it)", p.path),
        ));
    }
    let part = part_path(&target);
    let held = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if p.offset == 0 {
        // A fresh start replaces any earlier partial.
        let _ = fs::remove_file(&part);
    } else if p.offset != held {
        return Err(FileError::new(
            "bad_offset",
            format!(
                "this device holds {held} bytes of {} (send from offset {held})",
                p.path
            ),
        ));
    }
    if p.offset + bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(FileError::new(
            "too_large",
            format!("files over {MAX_FILE_BYTES} bytes aren't moved"),
        ));
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&part)
        .map_err(|e| FileError::io(&part, e))?;
    file.write_all(&bytes)
        .map_err(|e| FileError::io(&part, e))?;
    file.flush().map_err(|e| FileError::io(&part, e))?;
    drop(file);
    let received = p.offset + bytes.len() as u64;
    if !p.is_final {
        return Ok(json!({ "received": received }));
    }
    if let Some(want) = &p.sha256 {
        let (got, _) = sha256_of(&part)?;
        if !got.eq_ignore_ascii_case(want) {
            let _ = fs::remove_file(&part);
            return Err(FileError::new(
                "bad_hash",
                format!(
                    "{} arrived with a different sha256 (partial discarded)",
                    p.path
                ),
            ));
        }
    }
    if p.overwrite && target.exists() {
        // Windows can't rename over a file.
        fs::remove_file(&target).map_err(|e| FileError::io(&target, e))?;
    }
    fs::rename(&part, &target).map_err(|e| FileError::io(&target, e))?;
    Ok(json!({ "received": received, "final": true, "path": target.display().to_string() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, Policy) {
        let home = tempfile::tempdir().unwrap();
        let data = home.path().join(".local/share/zeron");
        fs::create_dir_all(&data).unwrap();
        let policy = Policy::new(home.path(), &data, []);
        (home, policy)
    }

    fn p(path: impl AsRef<Path>) -> PathParams {
        PathParams {
            path: path.as_ref().display().to_string(),
            cursor: None,
        }
    }

    fn code(r: Result<Value>) -> &'static str {
        r.unwrap_err().code
    }

    #[test]
    fn lists_reads_and_hashes_a_file_in_home() {
        let (home, policy) = setup();
        let file = home.path().join("a.bin");
        fs::write(&file, vec![7u8; 1_300_000]).unwrap();
        let listing = list(&policy, &p(home.path())).unwrap();
        assert!(
            listing["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["name"] == "a.bin" && e["size"] == 1_300_000)
        );
        let chunk = read(
            &policy,
            &ReadParams {
                path: file.display().to_string(),
                offset: 0,
                len: None,
            },
        )
        .unwrap();
        assert_eq!(chunk["nextOffset"], CHUNK_BYTES);
        assert_eq!(chunk["done"], false);
        let last = read(
            &policy,
            &ReadParams {
                path: file.display().to_string(),
                offset: 1_300_000 - 10,
                len: None,
            },
        )
        .unwrap();
        assert_eq!(last["done"], true);
        let h = hash(&policy, &p(&file)).unwrap();
        assert_eq!(h["size"], 1_300_000);
        assert_eq!(h["sha256"].as_str().unwrap().len(), 64);
    }

    #[test]
    fn a_read_over_the_chunk_size_is_too_large() {
        let (home, policy) = setup();
        let file = home.path().join("a");
        fs::write(&file, b"x").unwrap();
        let r = read(
            &policy,
            &ReadParams {
                path: file.display().to_string(),
                offset: 0,
                len: Some(CHUNK_BYTES + 1),
            },
        );
        assert_eq!(code(r), "too_large");
    }

    #[test]
    fn dotdot_outside_roots_and_relative_paths_are_refused() {
        let (home, policy) = setup();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("x"), b"x").unwrap();
        assert_eq!(
            code(stat(&policy, &p(outside.path().join("x")))),
            "no_grant"
        );
        let sneaky = format!(
            "{}/../{}/x",
            home.path().display(),
            outside.path().file_name().unwrap().to_string_lossy()
        );
        assert_eq!(
            code(stat(
                &policy,
                &PathParams {
                    path: sneaky,
                    cursor: None
                }
            )),
            "no_grant"
        );
        assert_eq!(
            code(stat(
                &policy,
                &PathParams {
                    path: "a/b".into(),
                    cursor: None
                }
            )),
            "denied"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_cannot_lead_out() {
        let (home, policy) = setup();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"s").unwrap();
        std::os::unix::fs::symlink(outside.path(), home.path().join("link")).unwrap();
        assert_eq!(
            code(read(
                &policy,
                &ReadParams {
                    path: home.path().join("link/secret").display().to_string(),
                    offset: 0,
                    len: None
                }
            )),
            "no_grant"
        );
        // …and the listing leaves the link out instead of failing.
        let names: Vec<_> = list(&policy, &p(home.path())).unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert!(!names.contains(&"link".to_string()));
    }

    #[test]
    fn the_deny_list_wins_over_home() {
        let (home, policy) = setup();
        for rel in [
            ".ssh/id_ed25519",
            ".aws/credentials",
            ".local/share/zeron/token",
            ".config/google-chrome/Default/Cookies",
            "proj/.git/config",
        ] {
            let f = home.path().join(rel);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(&f, b"secret").unwrap();
            assert_eq!(
                code(read(
                    &policy,
                    &ReadParams {
                        path: f.display().to_string(),
                        offset: 0,
                        len: None
                    }
                )),
                "denied",
                "{rel}"
            );
        }
        // Not listed from their parent either.
        let names: Vec<_> = list(&policy, &p(home.path())).unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert!(!names.contains(&".ssh".to_string()));
        assert!(names.contains(&"proj".to_string()));
        // Writes into a protected folder are refused too.
        let w = WritePartParams {
            transfer_id: "t".into(),
            path: home
                .path()
                .join(".ssh/authorized_keys")
                .display()
                .to_string(),
            offset: 0,
            data: B64.encode(b"x"),
            is_final: true,
            sha256: None,
            overwrite: false,
            make_parents: false,
        };
        assert_eq!(write_part(&policy, &w).unwrap_err().code, "denied");
    }

    fn part(
        path: &Path,
        offset: u64,
        data: &[u8],
        is_final: bool,
        sha: Option<String>,
        overwrite: bool,
    ) -> WritePartParams {
        WritePartParams {
            transfer_id: "t1".into(),
            path: path.display().to_string(),
            offset,
            data: B64.encode(data),
            is_final,
            sha256: sha,
            overwrite,
            make_parents: false,
        }
    }

    fn sha(data: &[u8]) -> String {
        hex(&Sha256::digest(data))
    }

    #[test]
    fn a_write_is_a_part_file_until_the_final_chunk_checks_it() {
        let (home, policy) = setup();
        let target = home.path().join("out.bin");
        let r = write_part(&policy, &part(&target, 0, b"hello ", false, None, false)).unwrap();
        assert_eq!(r["received"], 6);
        assert!(!target.exists());
        assert!(part_path(&target).exists());
        let r = write_part(
            &policy,
            &part(&target, 6, b"world", true, Some(sha(b"hello world")), false),
        )
        .unwrap();
        assert_eq!(r["final"], true);
        assert_eq!(fs::read(&target).unwrap(), b"hello world");
        assert!(!part_path(&target).exists());
    }

    #[test]
    fn a_wrong_offset_says_what_is_held_and_resume_continues_from_it() {
        let (home, policy) = setup();
        let target = home.path().join("out.bin");
        write_part(&policy, &part(&target, 0, b"abc", false, None, false)).unwrap();
        let err = write_part(&policy, &part(&target, 9, b"zzz", false, None, false)).unwrap_err();
        assert_eq!(err.code, "bad_offset");
        assert!(err.message.contains("holds 3 bytes"));
        write_part(
            &policy,
            &part(&target, 3, b"def", true, Some(sha(b"abcdef")), false),
        )
        .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"abcdef");
    }

    #[test]
    fn a_wrong_hash_discards_the_partial_and_leaves_no_target() {
        let (home, policy) = setup();
        let target = home.path().join("out.bin");
        let err = write_part(
            &policy,
            &part(&target, 0, b"abc", true, Some(sha(b"other")), false),
        )
        .unwrap_err();
        assert_eq!(err.code, "bad_hash");
        assert!(!target.exists());
        assert!(!part_path(&target).exists());
    }

    #[test]
    fn an_existing_target_needs_overwrite() {
        let (home, policy) = setup();
        let target = home.path().join("out.bin");
        fs::write(&target, b"old").unwrap();
        assert_eq!(
            write_part(&policy, &part(&target, 0, b"new", true, None, false))
                .unwrap_err()
                .code,
            "exists"
        );
        assert_eq!(fs::read(&target).unwrap(), b"old");
        write_part(&policy, &part(&target, 0, b"new", true, None, true)).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
    }

    #[test]
    fn a_listing_of_nothing_names_the_roots_and_pages_continue() {
        let (home, policy) = setup();
        for i in 0..(PAGE_ENTRIES + 20) {
            fs::write(home.path().join(format!("f{i:04}")), b"").unwrap();
        }
        let roots = list(&policy, &p("")).unwrap();
        assert_eq!(roots["entries"][0]["writable"], true);
        let first = list(&policy, &p(home.path())).unwrap();
        assert_eq!(first["entries"].as_array().unwrap().len(), PAGE_ENTRIES);
        let cursor = first["cursor"].as_str().unwrap().to_string();
        let second = list(
            &policy,
            &PathParams {
                path: home.path().display().to_string(),
                cursor: Some(cursor),
            },
        )
        .unwrap();
        assert!(second["entries"].as_array().unwrap().len() >= 20);
        assert!(second.get("cursor").is_none());
    }
}
