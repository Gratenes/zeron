//! Negotiated ACP image blocks for explicitly staged prompt attachments.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tokio::{fs::File, io::AsyncReadExt};

const IMAGE_CAP: usize = 5 * 1024 * 1024;
const TOTAL_CAP: usize = 20 * 1024 * 1024;

/// Preserve the authored prompt (including its path references) verbatim. This
/// helper has no session state: callers pass attachments only on their initial
/// prompt, and an empty slice for subsequent steering prompts.
pub(super) async fn content(
    text: String,
    attachments: &[String],
    supports_images: bool,
) -> Vec<Value> {
    let mut blocks = vec![json!({"type": "text", "text": text})];
    if !supports_images {
        return blocks;
    }
    let mut total = 0;
    for path in attachments {
        let cap = IMAGE_CAP.min(TOTAL_CAP - total);
        if cap == 0 {
            break;
        }
        let Ok(metadata) = tokio::fs::metadata(path).await else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > cap as u64 {
            continue;
        }
        let Ok(file) = File::open(path).await else {
            continue;
        };
        let mut bytes = Vec::new();
        // Do not trust metadata for the read bound: the file may have grown.
        if file
            .take(cap as u64 + 1)
            .read_to_end(&mut bytes)
            .await
            .is_err()
            || bytes.len() > cap
        {
            continue;
        }
        let Some(mime) = image_mime(&bytes) else {
            continue;
        };
        total += bytes.len();
        blocks.push(json!({"type": "image", "mimeType": mime, "data": STANDARD.encode(&bytes)}));
    }
    blocks
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // A complete 1×1 PNG, not just a signature or a renamed text file.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

    fn directory() -> tempfile::TempDir {
        tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap()
    }

    async fn attachment(dir: &Path, name: &str, bytes: &[u8]) -> String {
        let path = dir.join(name);
        tokio::fs::write(&path, bytes).await.unwrap();
        path.to_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn negotiated_png_preserves_exact_bytes_after_unchanged_text() {
        let dir = directory();
        let bytes = STANDARD.decode(PNG).unwrap();
        // File extension is deliberately misleading: bytes determine the MIME.
        let path = attachment(dir.path(), "staged.txt", &bytes).await;
        let text = format!("Inspect this image:\n{path}\nKeep this wording ✓");
        let blocks = content(text.clone(), &[path], true).await;
        assert_eq!(
            blocks,
            vec![
                json!({"type": "text", "text": text}),
                json!({"type": "image", "mimeType": "image/png", "data": PNG}),
            ]
        );
        assert_eq!(
            STANDARD
                .decode(blocks[1]["data"].as_str().unwrap())
                .unwrap(),
            bytes
        );
        assert_eq!(
            content("Steer only".into(), &[], true).await,
            vec![json!({"type": "text", "text": "Steer only"})]
        );
    }

    #[test]
    fn unsupported_capability_needs_no_tokio_runtime_or_filesystem_access() {
        // Tokio filesystem work would require a runtime. This path must return
        // synchronously without attempting to inspect or open any attachment.
        let blocks = futures::executor::block_on(content(
            "Keep /unreadable/image.png in the text".into(),
            &["/unreadable/image.png".into(), file!().into()],
            false,
        ));
        assert_eq!(
            blocks,
            vec![json!({
                "type": "text", "text": "Keep /unreadable/image.png in the text"
            })]
        );
    }

    #[tokio::test]
    async fn missing_invalid_nonfiles_and_resource_links_are_not_inlined() {
        let dir = directory();
        let invalid = attachment(dir.path(), "not-an-image.png", b"not image data").await;
        let truncated = attachment(dir.path(), "partial.png", b"\x89PNG").await;
        let empty = attachment(dir.path(), "empty.jpg", b"").await;
        let unlisted =
            attachment(dir.path(), "not-staged.png", &STANDARD.decode(PNG).unwrap()).await;
        let text = format!("This path stays text, not an implicit attachment: {unlisted}");
        let paths = vec![
            invalid,
            truncated,
            empty,
            dir.path().to_str().unwrap().into(),
            dir.path().join("missing.png").to_str().unwrap().into(),
            format!("file://{unlisted}"),
        ];
        assert_eq!(
            content(text.clone(), &paths, true).await,
            vec![json!({"type": "text", "text": text})]
        );
    }

    #[tokio::test]
    async fn signatures_not_extensions_select_supported_mime_types() {
        let dir = directory();
        for (index, (bytes, mime)) in [
            (b"\xff\xd8\xff\xe0jpeg".as_slice(), "image/jpeg"),
            (b"GIF87aimage".as_slice(), "image/gif"),
            (b"GIF89aimage".as_slice(), "image/gif"),
            (b"RIFF\x04\0\0\0WEBPimage".as_slice(), "image/webp"),
        ]
        .into_iter()
        .enumerate()
        {
            let path = attachment(dir.path(), &format!("{index}.bin"), bytes).await;
            let blocks = content(String::new(), &[path], true).await;
            assert_eq!(blocks.len(), 2);
            assert_eq!(blocks[0], json!({"type": "text", "text": ""}));
            assert_eq!(blocks[1]["mimeType"], mime);
            assert_eq!(
                STANDARD
                    .decode(blocks[1]["data"].as_str().unwrap())
                    .unwrap(),
                bytes
            );
        }
        let wav = attachment(dir.path(), "wrong.webp", b"RIFF\x04\0\0\0WAVEdata").await;
        assert_eq!(content("not an image".into(), &[wav], true).await.len(), 1);
    }

    #[tokio::test]
    async fn per_image_limit_rejects_oversize_without_truncating_it_into_an_image() {
        let dir = directory();
        let path = attachment(dir.path(), "oversize.png", &STANDARD.decode(PNG).unwrap()).await;
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .await
            .unwrap();
        file.set_len(IMAGE_CAP as u64 + 1).await.unwrap();
        assert_eq!(
            content("Keep the original path reference".into(), &[path], true).await,
            vec![json!({"type": "text", "text": "Keep the original path reference"})]
        );
    }

    #[tokio::test]
    async fn total_limit_skips_large_files_but_keeps_later_images_that_fit() {
        let dir = directory();
        let png = STANDARD.decode(PNG).unwrap();
        let small = attachment(dir.path(), "small.png", &png).await;
        let large = attachment(dir.path(), "large.png", &png).await;
        let almost = attachment(dir.path(), "almost.png", &png).await;
        for (path, size) in [(&large, IMAGE_CAP), (&almost, IMAGE_CAP - 2 * png.len())] {
            tokio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .unwrap()
                .set_len(size as u64)
                .await
                .unwrap();
        }
        let blocks = content(
            "All path references remain here".into(),
            &[
                large.clone(),
                large.clone(),
                large.clone(),
                almost,
                large,
                small.clone(),
                small.clone(),
                small,
            ],
            true,
        )
        .await;
        assert_eq!(blocks.len(), 7); // text + three full + almost + two small
        let sizes: Vec<_> = blocks[1..]
            .iter()
            .map(|block| {
                STANDARD
                    .decode(block["data"].as_str().unwrap())
                    .unwrap()
                    .len()
            })
            .collect();
        assert_eq!(
            sizes,
            [
                IMAGE_CAP,
                IMAGE_CAP,
                IMAGE_CAP,
                IMAGE_CAP - 2 * png.len(),
                png.len(),
                png.len()
            ]
        );
        assert_eq!(sizes.iter().sum::<usize>(), TOTAL_CAP);
    }
}
