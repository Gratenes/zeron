use rusqlite::{OptionalExtension, params};
use sha2::{Digest as _, Sha256};

use super::{PeerStore, PeerStoreError};

pub const MAX_ATTACHMENT_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_ATTACHMENT_CHUNK_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct CustodyMeta {
    pub sender: String,
    pub target: String,
    pub file_name: String,
    pub length: u64,
    pub digest: String,
    pub committed: bool,
    pub next_offset: u64,
}

impl PeerStore {
    pub(crate) fn init_attachment(
        &self,
        profile: &str,
        upload: &str,
        sender: &str,
        target: &str,
        file_name: &str,
        length: u64,
        digest: &str,
    ) -> Result<CustodyMeta, PeerStoreError> {
        if length > MAX_ATTACHMENT_BYTES || !valid_digest(digest) {
            return Err(PeerStoreError::Invalid("bad_metadata"));
        }
        let db = self.db();
        if let Some(meta) = load_meta(&db, profile, upload, sender, target)? {
            if meta.file_name != file_name || meta.length != length || meta.digest != digest {
                return Err(PeerStoreError::Invalid("metadata_conflict"));
            }
            return Ok(meta);
        }
        db.execute(
            "INSERT INTO attachment_uploads(profile,upload,sender,target,file_name,length,digest,committed,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,0,?8)",
            params![profile, upload, sender, target, file_name, length, digest, now_ms()],
        )?;
        Ok(CustodyMeta {
            sender: sender.into(),
            target: target.into(),
            file_name: file_name.into(),
            length,
            digest: digest.into(),
            committed: false,
            next_offset: 0,
        })
    }

    pub(crate) fn attachment_meta(
        &self,
        profile: &str,
        upload: &str,
        sender: &str,
        target: &str,
    ) -> Result<Option<CustodyMeta>, PeerStoreError> {
        load_meta(&self.db(), profile, upload, sender, target)
    }

    pub(crate) fn put_attachment_chunk(
        &self,
        profile: &str,
        upload: &str,
        sender: &str,
        target: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<CustodyMeta, PeerStoreError> {
        if bytes.is_empty() || bytes.len() > MAX_ATTACHMENT_CHUNK_BYTES {
            return Err(PeerStoreError::Invalid("bad_chunk"));
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let meta = load_meta(&tx, profile, upload, sender, target)?
            .ok_or(PeerStoreError::Invalid("not_found"))?;
        if meta.committed {
            return Err(PeerStoreError::Invalid("already_committed"));
        }
        if offset.saturating_add(bytes.len() as u64) > meta.length {
            return Err(PeerStoreError::Invalid("chunk_out_of_bounds"));
        }
        let prior: Option<Vec<u8>> = tx.query_row("SELECT bytes FROM attachment_chunks WHERE profile=?1 AND upload=?2 AND sender=?3 AND target=?4 AND offset=?5", params![profile,upload,sender,target,offset], |r| r.get(0)).optional()?;
        if let Some(prior) = prior {
            if prior != bytes {
                return Err(PeerStoreError::Invalid("chunk_conflict"));
            }
        } else {
            tx.execute("INSERT INTO attachment_chunks(profile,upload,sender,target,offset,bytes) VALUES(?1,?2,?3,?4,?5,?6)", params![profile,upload,sender,target,offset,bytes])?;
        }
        let next = contiguous_offset(&tx, profile, upload, sender, target)?;
        tx.commit()?;
        Ok(CustodyMeta {
            next_offset: next,
            ..meta
        })
    }

    pub(crate) fn commit_attachment(
        &self,
        profile: &str,
        upload: &str,
        sender: &str,
        target: &str,
    ) -> Result<CustodyMeta, PeerStoreError> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut meta = load_meta(&tx, profile, upload, sender, target)?
            .ok_or(PeerStoreError::Invalid("not_found"))?;
        if meta.committed {
            return Ok(meta);
        }
        let mut q=tx.prepare("SELECT offset,bytes FROM attachment_chunks WHERE profile=?1 AND upload=?2 AND sender=?3 AND target=?4 ORDER BY offset")?;
        let mut rows = q.query(params![profile, upload, sender, target])?;
        let mut bytes = Vec::with_capacity(meta.length as usize);
        let mut expected = 0u64;
        while let Some(row) = rows.next()? {
            let offset: u64 = row.get(0)?;
            let chunk: Vec<u8> = row.get(1)?;
            if offset != expected {
                return Err(PeerStoreError::Invalid("missing_chunk"));
            }
            expected += chunk.len() as u64;
            bytes.extend(chunk);
        }
        drop(rows);
        drop(q);
        if expected != meta.length {
            return Err(PeerStoreError::Invalid("missing_chunk"));
        }
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != meta.digest {
            return Err(PeerStoreError::Invalid("digest_mismatch"));
        }
        tx.execute("UPDATE attachment_uploads SET committed=1,committed_at=?5 WHERE profile=?1 AND upload=?2 AND sender=?3 AND target=?4",params![profile,upload,sender,target,now_ms()])?;
        tx.commit()?;
        meta.committed = true;
        meta.next_offset = meta.length;
        Ok(meta)
    }

    pub(crate) fn read_attachment(
        &self,
        profile: &str,
        upload: &str,
        sender: &str,
        target: &str,
        offset: u64,
    ) -> Result<Option<(CustodyMeta, Vec<u8>)>, PeerStoreError> {
        let db = self.db();
        let Some(meta) = load_meta(&db, profile, upload, sender, target)? else {
            return Ok(None);
        };
        if !meta.committed {
            return Err(PeerStoreError::Invalid("not_committed"));
        }
        if offset > meta.length {
            return Err(PeerStoreError::Invalid("range"));
        }
        let mut q=db.prepare("SELECT bytes FROM attachment_chunks WHERE profile=?1 AND upload=?2 AND sender=?3 AND target=?4 ORDER BY offset")?;
        let chunks = q.query_map(params![profile, upload, sender, target], |r| {
            r.get::<_, Vec<u8>>(0)
        })?;
        let mut all = Vec::with_capacity(meta.length as usize);
        for c in chunks {
            all.extend(c?);
        }
        Ok(Some((meta, all[offset as usize..].to_vec())))
    }
}

fn load_meta(
    db: &rusqlite::Connection,
    profile: &str,
    upload: &str,
    sender: &str,
    target: &str,
) -> Result<Option<CustodyMeta>, PeerStoreError> {
    let row=db.query_row("SELECT file_name,length,digest,committed FROM attachment_uploads WHERE profile=?1 AND upload=?2 AND sender=?3 AND target=?4",params![profile,upload,sender,target],|r|Ok((r.get::<_,String>(0)?,r.get::<_,u64>(1)?,r.get::<_,String>(2)?,r.get::<_,bool>(3)?))).optional()?;
    let Some((file_name, length, digest, committed)) = row else {
        return Ok(None);
    };
    let next = contiguous_offset(db, profile, upload, sender, target)?;
    Ok(Some(CustodyMeta {
        sender: sender.into(),
        target: target.into(),
        file_name,
        length,
        digest,
        committed,
        next_offset: next,
    }))
}
fn contiguous_offset(
    db: &rusqlite::Connection,
    profile: &str,
    upload: &str,
    sender: &str,
    target: &str,
) -> Result<u64, PeerStoreError> {
    let mut q=db.prepare("SELECT offset,LENGTH(bytes) FROM attachment_chunks WHERE profile=?1 AND upload=?2 AND sender=?3 AND target=?4 ORDER BY offset")?;
    let mut rows = q.query(params![profile, upload, sender, target])?;
    let mut next = 0;
    while let Some(r) = rows.next()? {
        let off: u64 = r.get(0)?;
        let len: u64 = r.get(1)?;
        if off != next {
            break;
        }
        next += len;
    }
    Ok(next)
}
fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && (!b.is_ascii_alphabetic() || b.is_ascii_lowercase()))
}
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
