//! Import clipboard history and pinned items from a Ditto database.
//!
//! Ditto stores clips in SQLite: `Main` holds one row per clip (`lID`, `lDate`
//! as Unix seconds, `mText`, `lDontAutoDelete` = keep/pinned, `bIsGroup`), and
//! `Data` holds the clipboard formats per clip (`lParentID` -> `Main.lID`,
//! `strClipBoardFormat`, `ooData` blob).
//!
//! Imported clips retain their dates and pinned state, and pass through Cubby's
//! encryption and content-hash boundary just like native captures.

use crate::clip_list::truncate_text_preview;
#[cfg(test)]
use crate::clip_list::TEXT_PREVIEW_CHAR_LIMIT;
use crate::clipboard::{build_clip_hash_material, CapturedFormat};
use crate::database::Database;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct DittoImportResult {
    pub total: usize,
    pub imported: usize,
    pub duplicates: usize,
    pub skipped_groups: usize,
    pub skipped_images: usize,
    pub skipped_empty: usize,
    pub skipped_malformed: usize,
    pub errors: Vec<String>,
    pub dry_run: bool,
}

fn is_image_format(format: &str) -> bool {
    let f = format.to_ascii_uppercase();
    f == "PNG" || f.starts_with("CF_DIB") || f == "CF_BITMAP" || f.contains("IMAGE")
}

/// Ditto's `CF_UNICODETEXT` blob is UTF-16LE, usually NUL-terminated.
fn decode_utf16le(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&u| u != 0)
        .collect();
    let text = String::from_utf16(&units).ok()?;
    let trimmed = text.trim_end_matches('\0');
    (!trimmed.trim().is_empty()).then(|| trimmed.to_string())
}

/// Decode the source's ANSI or OEM bytes with the locale recorded by Ditto.
/// When CF_LOCALE is absent, Windows' current ANSI/OEM page is the only known
/// fallback. Interpreting each byte as a Unicode scalar corrupts CP1252 and
/// double-byte code pages.
#[cfg(target_os = "windows")]
fn code_page(formats: &[(String, Vec<u8>)], oem: bool) -> u32 {
    use windows::Win32::Globalization::{
        GetACP, GetLocaleInfoW, GetOEMCP, LOCALE_IDEFAULTANSICODEPAGE, LOCALE_IDEFAULTCODEPAGE,
    };
    let fallback = unsafe {
        if oem {
            GetOEMCP()
        } else {
            GetACP()
        }
    };
    let locale = formats
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("CF_LOCALE"))
        .and_then(|(_, bytes)| bytes.get(..4))
        .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four bytes")));
    let Some(locale) = locale else {
        return fallback;
    };
    let kind = if oem {
        LOCALE_IDEFAULTCODEPAGE
    } else {
        LOCALE_IDEFAULTANSICODEPAGE
    };
    let mut value = [0u16; 16];
    let count = unsafe { GetLocaleInfoW(locale, kind, Some(&mut value)) };
    if count <= 1 {
        return fallback;
    }
    String::from_utf16(&value[..count as usize - 1])
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|&page| page != 0)
        .unwrap_or(fallback)
}

#[cfg(not(target_os = "windows"))]
fn code_page(_formats: &[(String, Vec<u8>)], _oem: bool) -> u32 {
    65001
}

#[cfg(target_os = "windows")]
fn decode_codepage(bytes: &[u8], page: u32) -> Option<String> {
    use windows::Win32::Globalization::{MultiByteToWideChar, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};
    let flags = MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0);
    let length = unsafe { MultiByteToWideChar(page, flags, bytes, None) };
    if length <= 0 {
        return None;
    }
    let mut wide = vec![0u16; length as usize];
    let written = unsafe { MultiByteToWideChar(page, flags, bytes, Some(&mut wide)) };
    (written == length)
        .then(|| String::from_utf16(&wide).ok())
        .flatten()
}

#[cfg(not(target_os = "windows"))]
fn decode_codepage(bytes: &[u8], _page: u32) -> Option<String> {
    String::from_utf8(bytes.to_vec()).ok()
}

fn decode_ansi(bytes: &[u8], page: u32) -> Option<String> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    if end == 0 {
        return None;
    }
    let text = decode_codepage(&bytes[..end], page)?;
    (!text.trim().is_empty()).then_some(text)
}

/// Pick the best text representation from a clip's formats, preferring unicode.
fn extract_text(formats: &[(String, Vec<u8>)]) -> Option<String> {
    let find = |name: &str| {
        formats
            .iter()
            .find(|(f, _)| f.eq_ignore_ascii_case(name))
            .map(|(_, d)| d)
    };
    if let Some(data) = find("CF_UNICODETEXT") {
        if let Some(text) = decode_utf16le(data) {
            return Some(text);
        }
    }
    for name in ["CF_TEXT", "CF_OEMTEXT"] {
        if let Some(data) = find(name) {
            if let Some(text) = decode_ansi(data, code_page(formats, name == "CF_OEMTEXT")) {
                return Some(text);
            }
        }
    }
    None
}

fn extract_rich_formats(formats: &[(String, Vec<u8>)]) -> Result<Vec<CapturedFormat>, String> {
    let mut rich = Vec::new();
    for (name, bytes) in formats {
        if name.eq_ignore_ascii_case("HTML Format") || name.eq_ignore_ascii_case("CF_HTML") {
            let end = bytes
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(bytes.len());
            let html = crate::cf_html::html_document_from_cf_html(&bytes[..end])
                .filter(|html| !html.trim().is_empty())
                .ok_or("invalid CF_HTML payload")?;
            if !rich
                .iter()
                .any(|format: &CapturedFormat| format.name == "html")
            {
                rich.push(CapturedFormat {
                    name: "html",
                    content: html.into_bytes(),
                });
            }
        } else if name.eq_ignore_ascii_case("Rich Text Format")
            || name.eq_ignore_ascii_case("CF_RTF")
            || name.eq_ignore_ascii_case("RTF")
        {
            let end = bytes
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(bytes.len());
            let rtf = &bytes[..end];
            if !rtf.starts_with(br"{\rtf") {
                return Err("invalid RTF payload".to_string());
            }
            if !rich
                .iter()
                .any(|format: &CapturedFormat| format.name == "rtf")
            {
                rich.push(CapturedFormat {
                    name: "rtf",
                    content: rtf.to_vec(),
                });
            }
        }
    }
    // Native capture hashes HTML before RTF even when the source advertises
    // them in the opposite order.
    rich.sort_by_key(|format| if format.name == "html" { 0 } else { 1 });
    Ok(rich)
}

struct ImportedImage {
    png: Vec<u8>,
    width: u32,
    height: u32,
}

struct DittoClipMeta {
    lid: i64,
    ldate: i64,
    pinned: bool,
}

/// Registered PNG is already the representation native capture would store.
/// A bitmap is decoded by the same DIB parser used for clipboard capture.
fn extract_image(formats: &[(String, Vec<u8>)]) -> Option<Result<ImportedImage, String>> {
    let png = formats
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("PNG"));
    let mut png_error = None;
    if let Some((_, bytes)) = png {
        match image::load_from_memory_with_format(bytes, image::ImageFormat::Png) {
            Ok(image) => {
                return Some(Ok(ImportedImage {
                    png: bytes.clone(),
                    width: image.width(),
                    height: image.height(),
                }));
            }
            Err(error) => png_error = Some(error.to_string()),
        }
    }

    for name in ["CF_DIBV5", "CF_DIB"] {
        if let Some((_, bytes)) = formats
            .iter()
            .find(|(format, _)| format.eq_ignore_ascii_case(name))
        {
            #[cfg(target_os = "windows")]
            {
                let image = match crate::clipboard::decode_clipboard_dib(bytes.clone()) {
                    Ok(image) => image,
                    Err(error) => {
                        png_error = Some(error);
                        continue;
                    }
                };
                let (width, height) = (image.width(), image.height());
                let mut png = std::io::Cursor::new(Vec::new());
                return Some(
                    image
                        .to_rgba8()
                        .write_to(&mut png, image_capture::ImageFormat::Png)
                        .map(|_| ImportedImage {
                            png: png.into_inner(),
                            width,
                            height,
                        })
                        .map_err(|error| error.to_string()),
                );
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = bytes;
                return Some(Err("DIB import requires Windows".to_string()));
            }
        }
    }
    png_error.map(Err)
}

/// Convert Ditto's Unix-seconds `lDate` to Cubby's `YYYY-MM-DD HH:MM:SS` UTC
/// text, matching the format SQLite's `CURRENT_TIMESTAMP` produces.
fn unix_to_datetime(seconds: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0)
        .unwrap_or_else(chrono::Utc::now)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

async fn import_image_clip(
    db: &Database,
    image: ImportedImage,
    clip: DittoClipMeta,
    dry_run: bool,
    planned: &mut std::collections::HashSet<String>,
    result: &mut DittoImportResult,
) {
    let DittoClipMeta { lid, ldate, pinned } = clip;
    let material = build_clip_hash_material("image", &image.png, std::iter::empty());
    let content_hash = db.crypto.keyed_hash(&material);
    let existing: Option<String> =
        match sqlx::query_scalar("SELECT uuid FROM clips WHERE content_hash = ?")
            .bind(&content_hash)
            .fetch_optional(&db.pool)
            .await
        {
            Ok(existing) => existing,
            Err(error) => {
                result
                    .errors
                    .push(format!("clip {lid}: duplicate check failed: {error}"));
                return;
            }
        };
    if existing.is_some() || planned.contains(&content_hash) {
        result.duplicates += 1;
        return;
    }
    let preview = match crate::clipboard::create_image_preview(&image.png) {
        Ok(preview) => preview,
        Err(error) => {
            result.skipped_malformed += 1;
            result
                .errors
                .push(format!("clip {lid}: invalid image: {error}"));
            return;
        }
    };
    if dry_run {
        result.imported += 1;
        planned.insert(content_hash);
        return;
    }

    let uuid = Uuid::new_v4().to_string();
    let encrypted_preview = match db.crypto.encrypt(&preview) {
        Ok(value) => value,
        Err(error) => {
            result
                .errors
                .push(format!("clip {lid}: preview encrypt failed: {error}"));
            return;
        }
    };
    let encrypted_label = match db.crypto.encrypt_text("[Image]") {
        Ok(value) => value,
        Err(error) => {
            result
                .errors
                .push(format!("clip {lid}: label encrypt failed: {error}"));
            return;
        }
    };
    let metadata = serde_json::json!({
        "width": image.width,
        "height": image.height,
        "format": "png",
        "size_bytes": image.png.len(),
    })
    .to_string();
    let encrypted_metadata = match db.crypto.encrypt_text(&metadata) {
        Ok(value) => value,
        Err(error) => {
            result
                .errors
                .push(format!("clip {lid}: metadata encrypt failed: {error}"));
            return;
        }
    };
    let mut transaction = match db.pool.begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            result
                .errors
                .push(format!("clip {lid}: transaction failed: {error}"));
            return;
        }
    };
    let created_at = unix_to_datetime(ldate);
    if let Err(error) = sqlx::query(
        "INSERT INTO clips (uuid, clip_type, content, text_preview, content_hash, folder_id, is_deleted, is_thumbnail, source_app, source_icon, metadata, ocr_status, created_at, last_accessed, is_pinned) VALUES (?, 'image', ?, ?, ?, NULL, 0, 0, NULL, NULL, ?, 'pending', ?, ?, ?)",
    )
    .bind(&uuid)
    .bind(&encrypted_preview)
    .bind(&encrypted_label)
    .bind(&content_hash)
    .bind(&encrypted_metadata)
    .bind(&created_at)
    .bind(&created_at)
    .bind(i64::from(pinned))
    .execute(&mut *transaction)
    .await
    {
        result.errors.push(format!("clip {lid}: insert failed: {error}"));
        return;
    }

    let path = match crate::image_persist::persist_full_image_file(
        &db.crypto,
        &db.image_dir,
        &uuid,
        &image.png,
    ) {
        Ok(path) => path,
        Err(error) => {
            result
                .errors
                .push(format!("clip {lid}: image save failed: {error}"));
            return;
        }
    };
    let insert_image = sqlx::query(
        "INSERT INTO clip_images (clip_uuid, full_content, file_path, file_size, storage_kind, mime_type, created_at) VALUES (?, x'', ?, ?, 'file', 'image/png', ?)",
    )
    .bind(&uuid)
    .bind(&path)
    .bind(image.png.len() as i64)
    .bind(&created_at)
    .execute(&mut *transaction)
    .await;
    if let Err(error) = insert_image {
        let _ = std::fs::remove_file(&path);
        result
            .errors
            .push(format!("clip {lid}: image index failed: {error}"));
        return;
    }
    match transaction.commit().await {
        Ok(()) => {
            result.imported += 1;
            planned.insert(content_hash);
        }
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            result
                .errors
                .push(format!("clip {lid}: commit failed: {error}"));
        }
    }
}

pub async fn import_from_ditto(
    db: &Database,
    ditto_db_path: &str,
    dry_run: bool,
) -> Result<DittoImportResult, String> {
    // Import from a private snapshot so a running/locked Ditto can't be read
    // mid-write. The guard removes the copy on every return path.
    struct TempDb(std::path::PathBuf);
    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let temp_path = std::env::temp_dir().join(format!("cubby-ditto-{}.db", Uuid::new_v4()));
    std::fs::copy(ditto_db_path, &temp_path)
        .map_err(|e| format!("Could not read the Ditto database: {e}"))?;
    let _temp_guard = TempDb(temp_path.clone());

    let options = SqliteConnectOptions::new()
        .filename(&temp_path)
        .read_only(true)
        .immutable(true);
    let ditto = SqlitePool::connect_with(options)
        .await
        .map_err(|e| format!("Could not open the Ditto database: {e}"))?;

    let clips = sqlx::query(
        "SELECT lID, lDate, mText, lDontAutoDelete, bIsGroup FROM Main ORDER BY lDate ASC",
    )
    .fetch_all(&ditto)
    .await
    .map_err(|e| format!("Could not read Ditto clips: {e}"))?;

    let mut result = DittoImportResult {
        dry_run,
        ..Default::default()
    };
    // Content hashes already planned this run, so intra-Ditto duplicates are
    // counted consistently in both dry-run previews and the real import.
    let mut planned: std::collections::HashSet<String> = std::collections::HashSet::new();

    for row in clips {
        result.total += 1;
        let fields = (
            row.try_get::<i64, _>("lID"),
            row.try_get::<i64, _>("lDate"),
            row.try_get::<Option<String>, _>("mText"),
            row.try_get::<i64, _>("lDontAutoDelete"),
            row.try_get::<i64, _>("bIsGroup"),
        );
        let (Ok(lid), Ok(ldate), Ok(mtext), Ok(dont_delete), Ok(is_group)) = fields else {
            result.skipped_malformed += 1;
            continue;
        };

        if is_group != 0 {
            result.skipped_groups += 1;
            continue;
        }

        let data_rows =
            match sqlx::query("SELECT strClipBoardFormat, ooData FROM Data WHERE lParentID = ?")
                .bind(lid)
                .fetch_all(&ditto)
                .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    result.skipped_malformed += 1;
                    result
                        .errors
                        .push(format!("clip {lid}: could not read formats: {error}"));
                    continue;
                }
            };

        let mut formats = Vec::with_capacity(data_rows.len());
        let mut malformed = false;
        for data_row in data_rows {
            match (
                data_row.try_get::<Option<String>, _>("strClipBoardFormat"),
                data_row.try_get::<Option<Vec<u8>>, _>("ooData"),
            ) {
                (Ok(Some(name)), Ok(Some(data))) if !name.is_empty() => formats.push((name, data)),
                _ => {
                    malformed = true;
                    break;
                }
            }
        }
        if malformed {
            result.skipped_malformed += 1;
            continue;
        }

        if let Some(image) = extract_image(&formats) {
            match image {
                Ok(image) => {
                    import_image_clip(
                        db,
                        image,
                        DittoClipMeta {
                            lid,
                            ldate,
                            pinned: dont_delete != 0,
                        },
                        dry_run,
                        &mut planned,
                        &mut result,
                    )
                    .await;
                }
                Err(_) => result.skipped_malformed += 1,
            }
            continue;
        }

        let rich = match extract_rich_formats(&formats) {
            Ok(rich) => rich,
            Err(_) => {
                result.skipped_malformed += 1;
                continue;
            }
        };

        let text = extract_text(&formats).or_else(|| mtext.filter(|t| !t.trim().is_empty()));

        let text = match text {
            Some(text) => text,
            None => {
                if formats.iter().any(|(f, _)| is_image_format(f)) {
                    result.skipped_images += 1;
                } else {
                    result.skipped_empty += 1;
                }
                continue;
            }
        };

        let hash_material = build_clip_hash_material(
            "text",
            text.as_bytes(),
            rich.iter()
                .map(|format| (format.name, format.content.as_slice())),
        );
        let content_hash = db.crypto.keyed_hash(&hash_material);

        let already: Option<String> =
            sqlx::query_scalar("SELECT uuid FROM clips WHERE content_hash = ?")
                .bind(&content_hash)
                .fetch_optional(&db.pool)
                .await
                .map_err(|e| format!("Could not check for an existing clip: {e}"))?;
        if already.is_some() || planned.contains(&content_hash) {
            result.duplicates += 1;
            continue;
        }

        if dry_run {
            result.imported += 1;
            planned.insert(content_hash);
            continue;
        }

        let encrypted_content = match db.crypto.encrypt(text.as_bytes()) {
            Ok(value) => value,
            Err(error) => {
                result
                    .errors
                    .push(format!("clip {lid}: encrypt failed: {error}"));
                continue;
            }
        };
        let encrypted_preview = match db.crypto.encrypt_text(&truncate_text_preview(&text)) {
            Ok(value) => value,
            Err(error) => {
                result
                    .errors
                    .push(format!("clip {lid}: preview encrypt failed: {error}"));
                continue;
            }
        };
        let created_at = unix_to_datetime(ldate);
        let is_pinned: i64 = (dont_delete != 0) as i64;
        let uuid = Uuid::new_v4().to_string();

        let metadata = (!rich.is_empty())
            .then(|| {
                serde_json::json!({
                    "formats": rich.iter().map(|format| format.name).collect::<Vec<_>>()
                })
                .to_string()
            })
            .map(|value| db.crypto.encrypt_text(&value))
            .transpose();
        let metadata = match metadata {
            Ok(value) => value,
            Err(error) => {
                result
                    .errors
                    .push(format!("clip {lid}: metadata encrypt failed: {error}"));
                continue;
            }
        };

        let mut transaction = match db.pool.begin().await {
            Ok(transaction) => transaction,
            Err(error) => {
                result
                    .errors
                    .push(format!("clip {lid}: transaction failed: {error}"));
                continue;
            }
        };

        let insert = sqlx::query(
            r#"
            INSERT INTO clips (uuid, clip_type, content, text_preview, content_hash, folder_id, is_deleted, is_thumbnail, source_app, source_icon, metadata, created_at, last_accessed, is_pinned)
            VALUES (?, 'text', ?, ?, ?, NULL, 0, 0, NULL, NULL, ?, ?, ?, ?)
            "#,
        )
        .bind(&uuid)
        .bind(&encrypted_content)
        .bind(&encrypted_preview)
        .bind(&content_hash)
        .bind(&metadata)
        .bind(&created_at)
        .bind(&created_at)
        .bind(is_pinned)
        .execute(&mut *transaction)
        .await;

        match insert {
            Ok(_) => {
                let mut format_error = None;
                for format in &rich {
                    let encrypted = match db.crypto.encrypt(&format.content) {
                        Ok(value) => value,
                        Err(error) => {
                            format_error = Some(format!("encrypt {}: {error}", format.name));
                            break;
                        }
                    };
                    if let Err(error) = sqlx::query(
                        "INSERT INTO clip_formats (clip_uuid, format, content) VALUES (?, ?, ?)",
                    )
                    .bind(&uuid)
                    .bind(format.name)
                    .bind(encrypted)
                    .execute(&mut *transaction)
                    .await
                    {
                        format_error = Some(format!("store {}: {error}", format.name));
                        break;
                    }
                }
                if let Some(error) = format_error {
                    result.errors.push(format!("clip {lid}: {error}"));
                    continue;
                }
                match transaction.commit().await {
                    Ok(()) => {
                        result.imported += 1;
                        planned.insert(content_hash);
                    }
                    Err(error) => result
                        .errors
                        .push(format!("clip {lid}: commit failed: {error}")),
                }
            }
            Err(error) => result
                .errors
                .push(format!("clip {lid}: insert failed: {error}")),
        }
    }

    ditto.close().await;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(text: &str) -> Vec<u8> {
        let mut bytes: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        bytes.extend_from_slice(&[0, 0]); // NUL terminator, as Ditto stores it
        bytes
    }

    fn tiny_dib() -> Vec<u8> {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER size
        dib.extend_from_slice(&2i32.to_le_bytes()); // width
        dib.extend_from_slice(&2i32.to_le_bytes()); // bottom-up height
        dib.extend_from_slice(&1u16.to_le_bytes()); // planes
        dib.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        dib.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        dib.extend_from_slice(&16u32.to_le_bytes()); // pixel data length
        dib.extend_from_slice(&[0u8; 16]); // resolution and color table counts
        dib.extend_from_slice(&[
            0, 0, 255, 255, // red
            0, 255, 0, 255, // green
            255, 0, 0, 255, // blue
            255, 255, 255, 255, // white
        ]);
        dib
    }

    fn tiny_png() -> Vec<u8> {
        let image = image::RgbaImage::from_fn(2, 2, |x, y| {
            image::Rgba([x as u8 * 80, y as u8 * 90, 140, 255])
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageOutputFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn decodes_unicode_text_and_strips_terminator() {
        assert_eq!(
            decode_utf16le(&utf16le("hello café 日本")).as_deref(),
            Some("hello café 日本")
        );
        assert_eq!(decode_utf16le(&utf16le("")), None);
        assert_eq!(decode_utf16le(&[0x41]), None); // odd length / too short
    }

    #[test]
    fn decodes_ansi_text() {
        assert_eq!(
            decode_ansi(b"powershell -File x\0", 1252).as_deref(),
            Some("powershell -File x")
        );
        assert_eq!(decode_ansi(b"\0", 1252), None);
        assert_eq!(decode_ansi(b"   ", 1252), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn decodes_text_with_its_recorded_windows_code_page() {
        let formats = vec![
            ("CF_LOCALE".to_string(), 0x0409u32.to_le_bytes().to_vec()),
            ("CF_TEXT".to_string(), b"caf\xe9\0".to_vec()),
        ];
        assert_eq!(code_page(&formats, false), 1252);
        assert_eq!(extract_text(&formats).as_deref(), Some("café"));
    }

    #[test]
    fn prefers_unicode_over_ansi() {
        let formats = vec![
            ("CF_TEXT".to_string(), b"ansi\0".to_vec()),
            ("CF_UNICODETEXT".to_string(), utf16le("unicode")),
        ];
        assert_eq!(extract_text(&formats).as_deref(), Some("unicode"));
    }

    #[test]
    fn recognizes_image_formats() {
        assert!(is_image_format("PNG"));
        assert!(is_image_format("CF_DIB"));
        assert!(is_image_format("CF_DIBV5"));
        assert!(!is_image_format("CF_UNICODETEXT"));
        assert!(!is_image_format("HTML Format"));
    }

    #[test]
    fn converts_unix_seconds_to_sqlite_utc() {
        // 2021-01-01 00:00:00 UTC
        assert_eq!(unix_to_datetime(1609459200), "2021-01-01 00:00:00");
    }

    #[test]
    fn preview_is_truncated_by_chars_not_bytes() {
        let long = "é".repeat(TEXT_PREVIEW_CHAR_LIMIT + 100);
        assert_eq!(
            truncate_text_preview(&long).chars().count(),
            TEXT_PREVIEW_CHAR_LIMIT
        );
    }

    #[tokio::test]
    async fn imports_text_clips_with_dates_pins_and_dedup() {
        use crate::crypto::CryptoManager;
        use crate::database::Database;
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
        use std::sync::Arc;

        // Cubby destination DB (in-memory) with the real clips schema.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("cubby db opens");
        sqlx::query(
            r#"
            CREATE TABLE clips (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                uuid TEXT NOT NULL UNIQUE,
                clip_type TEXT NOT NULL,
                content BLOB NOT NULL,
                text_preview TEXT,
                content_hash TEXT NOT NULL,
                folder_id INTEGER,
                is_deleted INTEGER DEFAULT 0,
                is_thumbnail INTEGER NOT NULL DEFAULT 0,
                source_app TEXT,
                source_icon TEXT,
                metadata TEXT,
                ocr_status TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                last_accessed DATETIME DEFAULT CURRENT_TIMESTAMP,
                is_pinned INTEGER NOT NULL DEFAULT 0
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("clips table");
        sqlx::query("CREATE TABLE clip_formats (clip_uuid TEXT NOT NULL, format TEXT NOT NULL, content BLOB NOT NULL, PRIMARY KEY (clip_uuid, format))")
            .execute(&pool)
            .await
            .expect("formats table");
        sqlx::query("CREATE TABLE clip_images (clip_uuid TEXT PRIMARY KEY, full_content BLOB NOT NULL, file_path TEXT, file_size INTEGER, storage_kind TEXT, mime_type TEXT, created_at DATETIME)")
            .execute(&pool)
            .await
            .expect("images table");
        let db = Database {
            pool,
            crypto: Arc::new(CryptoManager::ephemeral()),
            image_dir: std::env::temp_dir().join(format!("cubby-ditto-test-{}", Uuid::new_v4())),
            search_index: Arc::new(crate::search_index::SearchIndex::default()),
        };

        // Synthetic Ditto source DB (temp file; the importer opens it read-only).
        let ditto_path = std::env::temp_dir().join(format!("ditto-src-{}.db", Uuid::new_v4()));
        {
            let ditto = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&ditto_path)
                        .create_if_missing(true)
                        .journal_mode(SqliteJournalMode::Delete),
                )
                .await
                .expect("ditto db creates");
            sqlx::query("CREATE TABLE Main(lID INTEGER PRIMARY KEY, lDate INTEGER, mText TEXT, lDontAutoDelete INTEGER, bIsGroup INTEGER)")
                .execute(&ditto).await.unwrap();
            sqlx::query("CREATE TABLE Data(lID INTEGER PRIMARY KEY, lParentID INTEGER, strClipBoardFormat TEXT, ooData BLOB)")
                .execute(&ditto).await.unwrap();

            async fn main_row(
                p: &sqlx::SqlitePool,
                id: i64,
                date: i64,
                mtext: &str,
                pin: i64,
                group: i64,
            ) {
                sqlx::query(
                    "INSERT INTO Main(lID,lDate,mText,lDontAutoDelete,bIsGroup) VALUES (?,?,?,?,?)",
                )
                .bind(id)
                .bind(date)
                .bind(mtext)
                .bind(pin)
                .bind(group)
                .execute(p)
                .await
                .unwrap();
            }
            async fn data_row(p: &sqlx::SqlitePool, parent: i64, fmt: &str, blob: Vec<u8>) {
                sqlx::query("INSERT INTO Data(lParentID,strClipBoardFormat,ooData) VALUES (?,?,?)")
                    .bind(parent)
                    .bind(fmt)
                    .bind(blob)
                    .execute(p)
                    .await
                    .unwrap();
            }

            main_row(&ditto, 1, 1609459200, "reset steps", 1, 0).await; // pinned unicode
            data_row(&ditto, 1, "CF_UNICODETEXT", utf16le("reset password steps")).await;
            main_row(&ditto, 2, 1609545600, "isql", 0, 0).await; // ansi text
            data_row(&ditto, 2, "CF_TEXT", b"isql -u SYSDBA\0".to_vec()).await;
            main_row(&ditto, 3, 1609600000, "My Group", 0, 1).await; // group -> skip
            main_row(&ditto, 4, 1609700000, "", 0, 0).await; // DIB image
            data_row(&ditto, 4, "CF_DIB", tiny_dib()).await;
            main_row(&ditto, 5, 1609800000, "https://example.com/kb/42", 0, 0).await; // mText only
            main_row(&ditto, 6, 1609900000, "dupe", 0, 0).await; // duplicate of #1
            data_row(&ditto, 6, "CF_UNICODETEXT", utf16le("reset password steps")).await;
            main_row(&ditto, 7, 1610000000, "", 0, 0).await; // malformed format row
            sqlx::query("INSERT INTO Data(lParentID,strClipBoardFormat,ooData) VALUES (7,'CF_UNICODETEXT',NULL)")
                .execute(&ditto).await.unwrap();
            main_row(&ditto, 8, 1610100000, "still valid", 0, 0).await;
            data_row(&ditto, 8, "CF_UNICODETEXT", utf16le("styled table")).await;
            data_row(
                &ditto,
                8,
                "HTML Format",
                crate::cf_html::to_cf_html("<b>styled table</b>").into_bytes(),
            )
            .await;
            data_row(
                &ditto,
                8,
                "Rich Text Format",
                br"{\rtf1\ansi styled table}".to_vec(),
            )
            .await;
            main_row(&ditto, 9, 1610200000, "", 0, 0).await; // PNG beats broken DIB
            data_row(&ditto, 9, "CF_DIB", vec![0u8; 40]).await;
            data_row(&ditto, 9, "PNG", tiny_png()).await;
            main_row(&ditto, 10, 1610300000, "", 0, 0).await; // unsupported bitmap
            data_row(&ditto, 10, "CF_BITMAP", vec![0u8; 4]).await;

            ditto.close().await;
        }

        // A fresh dry run counts intra-Ditto duplicates (clip 6 == clip 1) once.
        let initial_dry = import_from_ditto(&db, ditto_path.to_str().unwrap(), true)
            .await
            .expect("dry run");
        assert_eq!(initial_dry.imported, 6, "clip 6 is a dup of clip 1");
        assert_eq!(initial_dry.duplicates, 1);
        assert_eq!(initial_dry.skipped_malformed, 1);

        let result = import_from_ditto(&db, ditto_path.to_str().unwrap(), false)
            .await
            .expect("import runs");
        assert_eq!(result.total, 10);
        assert_eq!(result.skipped_groups, 1);
        assert_eq!(result.skipped_images, 1);
        assert_eq!(
            result.imported, 6,
            "valid clips after a malformed row still import"
        );
        assert_eq!(result.duplicates, 1);
        assert_eq!(result.skipped_malformed, 1);
        assert!(result.errors.is_empty(), "errors: {:?}", result.errors);

        // The pinned clip decrypts to its original text and keeps its Ditto date.
        let (content, created): (Vec<u8>, String) = sqlx::query_as(
            "SELECT content, created_at FROM clips WHERE is_pinned = 1 AND clip_type = 'text'",
        )
        .fetch_one(&db.pool)
        .await
        .expect("one pinned clip");
        assert_eq!(
            db.crypto.decrypt(&content).unwrap(),
            b"reset password steps"
        );
        assert_eq!(created, "2021-01-01 00:00:00");

        let rich_rows: Vec<(String, Vec<u8>)> = sqlx::query_as(
            "SELECT format, content FROM clip_formats WHERE clip_uuid = (SELECT uuid FROM clips WHERE created_at = '2021-01-08 10:00:00') ORDER BY format",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(rich_rows.len(), 2);
        assert_eq!(rich_rows[0].0, "html");
        let html = db.crypto.decrypt(&rich_rows[0].1).unwrap();
        assert!(!html.starts_with(b"Version:"), "CF_HTML header is stripped");
        assert!(html.windows(15).any(|part| part == b"<b>styled table"));
        assert_eq!(rich_rows[1].0, "rtf");
        assert_eq!(
            db.crypto.decrypt(&rich_rows[1].1).unwrap(),
            br"{\rtf1\ansi styled table}"
        );

        let images: Vec<(Vec<u8>, String)> = sqlx::query_as(
            "SELECT clips.content, clip_images.file_path FROM clips JOIN clip_images ON clips.uuid = clip_images.clip_uuid ORDER BY clips.created_at",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(images.len(), 2);
        for (encrypted_thumbnail, path) in images {
            assert!(path.ends_with(".cubby"));
            let thumbnail = db.crypto.decrypt(&encrypted_thumbnail).unwrap();
            assert_eq!(&thumbnail[..8], b"\x89PNG\r\n\x1a\n");
            let encrypted_original = std::fs::read(path).unwrap();
            let original = db.crypto.decrypt(&encrypted_original).unwrap();
            assert_eq!(&original[..8], b"\x89PNG\r\n\x1a\n");
        }

        // Re-running as a dry run against the now-populated DB imports nothing.
        let dry = import_from_ditto(&db, ditto_path.to_str().unwrap(), true)
            .await
            .unwrap();
        assert_eq!(dry.imported, 0);
        assert_eq!(dry.duplicates, 7);

        let _ = std::fs::remove_file(&ditto_path);
        let _ = std::fs::remove_dir_all(&db.image_dir);
    }
}
