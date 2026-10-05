//! メタデータの読み取り・編集・削除。
//!
//! - PNG: tEXt / iTXt チャンクの読み書き（画像生成AIの parameters 等を想定）。
//!   圧縮チャンク（zTXt・圧縮iTXt）は読み取り表示のみとし、編集時はバイト列を保持する。
//! - JPEG: EXIF の読み取りと、APPn/COM マーカー除去による削除（再圧縮なし）。
//! - JPEG の EXIF 書き換えは未対応（IFD 再構築が必要なため将来対応）。

use std::io::Cursor;

use serde::{Deserialize, Serialize};

use crate::convert::{ConvertResult, OutputFormat, encode, normalize_orientation};

const PNG_MAGIC: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

#[derive(Debug, Serialize)]
pub struct ExifEntry {
    pub tag: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct PngTextInfo {
    pub keyword: String,
    pub text: String,
    pub compressed: bool,
}

#[derive(Debug, Serialize)]
pub struct MetadataInfo {
    pub format: String,
    pub exif: Vec<ExifEntry>,
    pub png_text: Vec<PngTextInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PngTextEntry {
    pub keyword: String,
    pub text: String,
}

struct PngChunk {
    ty: [u8; 4],
    data: Vec<u8>,
}

fn latin1_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

fn string_to_latin1(s: &str) -> Option<Vec<u8>> {
    s.chars()
        .map(|c| u32::from(c).try_into().map_err(|_| ()))
        .collect::<Result<Vec<u8>, ()>>()
        .ok()
}

fn parse_png(bytes: &[u8]) -> Result<Vec<PngChunk>, String> {
    if bytes.len() < 8 || bytes[..8] != PNG_MAGIC {
        return Err("PNG 形式ではありません。".to_string());
    }
    let mut chunks = Vec::new();
    let mut pos = 8;
    loop {
        if pos + 8 > bytes.len() {
            return Err("PNG チャンクが壊れています。".to_string());
        }
        let len =
            u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                as usize;
        let ty = [bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]];
        let end = pos + 8 + len + 4;
        if end > bytes.len() {
            return Err("PNG チャンクが壊れています。".to_string());
        }
        chunks.push(PngChunk {
            ty,
            data: bytes[pos + 8..pos + 8 + len].to_vec(),
        });
        if &ty == b"IEND" {
            break;
        }
        pos = end;
    }
    Ok(chunks)
}

fn write_png(chunks: &[PngChunk]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1024);
    out.extend_from_slice(&PNG_MAGIC);
    for c in chunks {
        out.extend_from_slice(&(c.data.len() as u32).to_be_bytes());
        out.extend_from_slice(&c.ty);
        out.extend_from_slice(&c.data);
        let mut h = crc32fast::Hasher::new();
        h.update(&c.ty);
        h.update(&c.data);
        out.extend_from_slice(&h.finalize().to_be_bytes());
    }
    out
}

fn split_keyword(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let nul = data.iter().position(|&b| b == 0)?;
    Some((&data[..nul], &data[nul + 1..]))
}

/// iTXt を解釈する。戻り値は (keyword, text, 圧縮済みか)。
fn parse_itxt(data: &[u8]) -> Option<(String, String, bool)> {
    let (keyword, rest) = split_keyword(data)?;
    if rest.len() < 2 {
        return None;
    }
    let compressed = rest[0] == 1;
    let mut parts = rest[2..].split(|&b| b == 0);
    let _lang = parts.next()?;
    let _translated = parts.next()?;
    let text_bytes: Vec<u8> = parts.collect::<Vec<&[u8]>>().join(&0);
    if compressed {
        return Some((latin1_to_string(keyword), String::new(), true));
    }
    Some((
        latin1_to_string(keyword),
        String::from_utf8_lossy(&text_bytes).into_owned(),
        false,
    ))
}

fn read_png_text(chunks: &[PngChunk]) -> Vec<PngTextInfo> {
    let mut out = Vec::new();
    for c in chunks {
        match &c.ty {
            b"tEXt" => {
                if let Some((keyword, text)) = split_keyword(&c.data) {
                    out.push(PngTextInfo {
                        keyword: latin1_to_string(keyword),
                        text: latin1_to_string(text),
                        compressed: false,
                    });
                }
            }
            b"zTXt" => {
                if let Some((keyword, _)) = split_keyword(&c.data) {
                    out.push(PngTextInfo {
                        keyword: latin1_to_string(keyword),
                        text: String::new(),
                        compressed: true,
                    });
                }
            }
            b"iTXt" => {
                if let Some((keyword, text, compressed)) = parse_itxt(&c.data) {
                    out.push(PngTextInfo {
                        keyword,
                        text,
                        compressed,
                    });
                }
            }
            _ => {}
        }
    }
    out
}

fn make_text_chunk(keyword: &str, text: &str) -> Result<PngChunk, String> {
    let kw = keyword.trim();
    if kw.is_empty() || kw.len() > 79 || kw.contains('\0') {
        return Err(format!("キーワードが不正です: {keyword}"));
    }
    if text.contains('\0') {
        return Err("テキストに NUL 文字は使えません。".to_string());
    }
    // keyword・text とも Latin-1 なら tEXt、そうでなければ iTXt（UTF-8）。
    if let (Some(k), Some(t)) = (string_to_latin1(kw), string_to_latin1(text)) {
        let mut data = k;
        data.push(0);
        data.extend_from_slice(&t);
        return Ok(PngChunk {
            ty: *b"tEXt",
            data,
        });
    }
    if !kw.is_ascii() {
        return Err("キーワードは ASCII で指定してください。".to_string());
    }
    let mut data = kw.as_bytes().to_vec();
    data.push(0); // keyword NUL
    data.push(0); // compression flag = 0
    data.push(0); // compression method
    data.push(0); // langtag (empty + NUL)
    data.push(0); // translated keyword (empty + NUL)
    data.extend_from_slice(text.as_bytes());
    Ok(PngChunk {
        ty: *b"iTXt",
        data,
    })
}

fn value_to_string(v: &exif::Value) -> String {
    const LIMIT: usize = 300;
    let s = match v {
        exif::Value::Ascii(parts) => parts
            .iter()
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect::<Vec<_>>()
            .join(" "),
        other => format!("{other:?}"),
    };
    if s.len() > LIMIT {
        format!("{}…", &s[..LIMIT])
    } else {
        s
    }
}

fn read_exif(bytes: &[u8]) -> Vec<ExifEntry> {
    let Ok(exif) = exif::Reader::new().read_from_container(&mut Cursor::new(bytes)) else {
        return Vec::new();
    };
    exif.fields()
        .map(|f| ExifEntry {
            tag: format!("{:?}", f.tag),
            value: value_to_string(&f.value),
        })
        .collect()
}

fn read_metadata_blocking(input: &str) -> Result<MetadataInfo, String> {
    let bytes = std::fs::read(input).map_err(|e| format!("入力ファイルを開けません: {e}"))?;
    if bytes.len() >= 8 && bytes[..8] == PNG_MAGIC {
        let chunks =
            parse_png(&bytes).map_err(|e| format!("PNG の解析に失敗しました: {e}"))?;
        return Ok(MetadataInfo {
            format: "png".to_string(),
            exif: Vec::new(),
            png_text: read_png_text(&chunks),
        });
    }
    if bytes.len() >= 2 && bytes[..2] == [0xFF, 0xD8] {
        return Ok(MetadataInfo {
            format: "jpeg".to_string(),
            exif: read_exif(&bytes),
            png_text: Vec::new(),
        });
    }
    Err("PNG / JPEG のみ対応しています。".to_string())
}

/// PNG テキストチャンクを書き換える。入力が PNG 以外の場合は
/// 先に PNG へ変換してから書き込む。
fn save_metadata_with_format_blocking(
    input: &str,
    output: &str,
    format: &str,
    quality: u8,
    entries: &[PngTextEntry],
) -> Result<ConvertResult, String> {
    let format = OutputFormat::parse(format)?;
    let quality = quality.clamp(1, 100);
    let bytes = std::fs::read(input).map_err(|e| format!("入力ファイルを開けません: {e}"))?;
    let out = if format == OutputFormat::Png {
        let png_bytes = if bytes.len() >= 8 && bytes[..8] == PNG_MAGIC {
            bytes.clone()
        } else {
            let img = image::load_from_memory(&bytes)
                .map_err(|e| format!("画像の読み込みに失敗しました: {e}"))?;
            let img = normalize_orientation(img, &bytes);
            encode(&img, OutputFormat::Png, quality)?
        };
        apply_png_text(&png_bytes, entries)?
    } else {
        let img = image::load_from_memory(&bytes)
            .map_err(|e| format!("画像の読み込みに失敗しました: {e}"))?;
        let img = normalize_orientation(img, &bytes);
        encode(&img, format, quality)?
    };
    std::fs::write(output, &out).map_err(|e| format!("出力ファイルの書き込みに失敗しました: {e}"))?;
    let img =
        image::load_from_memory(&out).map_err(|e| format!("出力画像の検証に失敗しました: {e}"))?;
    Ok(ConvertResult {
        output_path: output.to_string(),
        format: format.extension().to_string(),
        width: img.width(),
        height: img.height(),
        input_bytes: bytes.len() as u64,
        output_bytes: out.len() as u64,
    })
}

/// PNG バイト列のテキストチャンクを entries で置換する。
fn apply_png_text(png_bytes: &[u8], entries: &[PngTextEntry]) -> Result<Vec<u8>, String> {
    let chunks = parse_png(png_bytes).map_err(|e| format!("PNG の解析に失敗しました: {e}"))?;
    let mut kept = Vec::new();
    let mut dropped_text = 0;
    for c in &chunks {
        match &c.ty {
            // tEXt と非圧縮 iTXt は置換対象。圧縮済みは保持する。
            b"tEXt" => dropped_text += 1,
            b"iTXt" => {
                let compressed = parse_itxt(&c.data).map(|(_, _, c)| c).unwrap_or(true);
                if compressed {
                    kept.push(PngChunk {
                        ty: c.ty,
                        data: c.data.clone(),
                    });
                } else {
                    dropped_text += 1;
                }
            }
            b"zTXt" => kept.push(PngChunk {
                ty: c.ty,
                data: c.data.clone(),
            }),
            _ => kept.push(PngChunk {
                ty: c.ty,
                data: c.data.clone(),
            }),
        }
    }
    let mut fresh = Vec::with_capacity(entries.len());
    for e in entries {
        fresh.push(make_text_chunk(&e.keyword, &e.text)?);
    }
    // 新規チャンクは IEND の直前に挿入する。
    let iend = kept
        .iter()
        .position(|c| &c.ty == b"IEND")
        .ok_or_else(|| "PNG に IEND がありません。".to_string())?;
    let mut kept_tail = kept;
    let tail = kept_tail.split_off(iend);
    let mut merged = Vec::with_capacity(kept_tail.len() + tail.len() + fresh.len());
    merged.extend(kept_tail);
    merged.extend(fresh);
    merged.extend(tail);
    let _ = dropped_text;
    Ok(write_png(&merged))
}

/// JPEG の APPn / COM マーカーを取り除く（再圧縮なし）。
fn strip_jpeg(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 2 || bytes[..2] != [0xFF, 0xD8] {
        return Err("JPEG 形式ではありません。".to_string());
    }
    let mut out = vec![0xFF, 0xD8];
    let mut pos = 2;
    // スタンドアロンマーカー（長さ field なし）。
    let standalone = |m: u8| m == 0xD8 || m == 0xD9 || (0xD0..=0xD7).contains(&m) || m == 0x01;
    while pos + 1 < bytes.len() {
        if bytes[pos] != 0xFF {
            return Err("JPEG マーカーが壊れています。".to_string());
        }
        let mut m = pos + 1;
        while m < bytes.len() && bytes[m] == 0xFF {
            m += 1; // fill bytes
        }
        if m >= bytes.len() {
            break;
        }
        let marker = bytes[m];
        if marker == 0x00 {
            return Err("JPEG マーカーが壊れています。".to_string());
        }
        if standalone(marker) {
            out.extend_from_slice(&[0xFF, marker]);
            pos = m + 1;
            if marker == 0xD9 {
                break;
            }
            continue;
        }
        if m + 2 >= bytes.len() {
            return Err("JPEG マーカーが壊れています。".to_string());
        }
        let len = u16::from_be_bytes([bytes[m + 1], bytes[m + 2]]) as usize;
        if len < 2 || m + 1 + len > bytes.len() {
            return Err("JPEG マーカーが壊れています。".to_string());
        }
        if marker == 0xDA {
            // SOS 以降は EOI まで丸ごとコピーする。
            out.extend_from_slice(&bytes[pos..]);
            break;
        }
        let drop = (0xE0..=0xEF).contains(&marker) || marker == 0xFE;
        if !drop {
            out.extend_from_slice(&bytes[pos..m + 1 + len]);
        }
        pos = m + 1 + len;
    }
    Ok(out)
}

fn remove_metadata_blocking(input: &str, output: &str) -> Result<ConvertResult, String> {
    let bytes = std::fs::read(input).map_err(|e| format!("入力ファイルを開けません: {e}"))?;
    if bytes.len() >= 8 && bytes[..8] == PNG_MAGIC {
        let chunks = parse_png(&bytes).map_err(|e| format!("PNG の解析に失敗しました: {e}"))?;
        let kept: Vec<PngChunk> = chunks
            .into_iter()
            .filter(|c| !matches!(&c.ty, b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf"))
            .map(|c| PngChunk {
                ty: c.ty,
                data: c.data,
            })
            .collect();
        let out = write_png(&kept);
        std::fs::write(output, &out)
            .map_err(|e| format!("出力ファイルの書き込みに失敗しました: {e}"))?;
        let img = image::load_from_memory(&out)
            .map_err(|e| format!("出力画像の検証に失敗しました: {e}"))?;
        return Ok(ConvertResult {
            output_path: output.to_string(),
            format: "png".to_string(),
            width: img.width(),
            height: img.height(),
            input_bytes: bytes.len() as u64,
            output_bytes: out.len() as u64,
        });
    }
    if bytes.len() >= 2 && bytes[..2] == [0xFF, 0xD8] {
        let out = strip_jpeg(&bytes)?;
        std::fs::write(output, &out)
            .map_err(|e| format!("出力ファイルの書き込みに失敗しました: {e}"))?;
        let img = image::load_from_memory(&out)
            .map_err(|e| format!("出力画像の検証に失敗しました: {e}"))?;
        return Ok(ConvertResult {
            output_path: output.to_string(),
            format: "jpeg".to_string(),
            width: img.width(),
            height: img.height(),
            input_bytes: bytes.len() as u64,
            output_bytes: out.len() as u64,
        });
    }
    Err("PNG / JPEG のみ対応しています。".to_string())
}

/// メタデータを読み取る。
#[tauri::command]
pub async fn read_metadata(input: String) -> Result<MetadataInfo, String> {
    tauri::async_runtime::spawn_blocking(move || read_metadata_blocking(&input))
        .await
        .map_err(|e| format!("処理が中断されました: {e}"))?
}

/// メタデータを保存する。PNG 出力時はテキスト情報を保持・更新し、
/// PNG 以外への変換時はメタデータを落として画像のみ変換する。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn save_metadata_with_format(
    input: String,
    output: String,
    format: String,
    quality: u8,
    entries: Vec<PngTextEntry>,
) -> Result<ConvertResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        save_metadata_with_format_blocking(&input, &output, &format, quality, &entries)
    })
    .await
    .map_err(|e| format!("処理が中断されました: {e}"))?
}

/// メタデータを削除する（再圧縮なし）。
#[tauri::command]
pub async fn remove_metadata(input: String, output: String) -> Result<ConvertResult, String> {
    tauri::async_runtime::spawn_blocking(move || remove_metadata_blocking(&input, &output))
        .await
        .map_err(|e| format!("処理が中断されました: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn fixture_png() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pixelt-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        let path = dir.join(format!("input-{:?}.png", std::thread::current().id()));
        img.save(&path).unwrap();
        path
    }

    fn fixture_jpeg() -> std::path::PathBuf {
        use image::codecs::jpeg::JpegEncoder;
        use image::ImageEncoder;
        let dir = std::env::temp_dir().join(format!("pixelt-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 64]));
        let path = dir.join(format!("input-{:?}.jpg", std::thread::current().id()));
        let mut buf = Vec::new();
        JpegEncoder::new_with_quality(&mut buf, 85)
            .write_image(
                img.as_raw(),
                64,
                48,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        std::fs::write(&path, &buf).unwrap();
        path
    }

    #[test]
    fn writes_and_reads_png_text() {
        let input = fixture_png();
        let output = input.with_file_name("meta.png");
        let entries = vec![
            PngTextEntry {
                keyword: "parameters".to_string(),
                text: "steps: 20, cfg: 7".to_string(),
            },
            PngTextEntry {
                keyword: "comment-ja".to_string(),
                text: "日本語プロンプト".to_string(),
            },
        ];
        let r = save_metadata_with_format_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "png",
            85,
            &entries,
        )
        .unwrap();
        assert_eq!(r.format, "png");
        let meta =
            read_metadata_blocking(output.to_str().unwrap()).unwrap();
        assert_eq!(meta.png_text.len(), 2);
        assert_eq!(meta.png_text[0].keyword, "parameters");
        assert_eq!(meta.png_text[0].text, "steps: 20, cfg: 7");
        assert!(!meta.png_text[0].compressed);
        assert_eq!(meta.png_text[1].text, "日本語プロンプト");
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn strips_jpeg_metadata_losslessly() {
        let input = fixture_jpeg();
        let output = input.with_file_name("stripped.jpg");
        let r =
            remove_metadata_blocking(input.to_str().unwrap(), output.to_str().unwrap()).unwrap();
        assert_eq!(r.format, "jpeg");
        let bytes = std::fs::read(&output).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
        assert_eq!(&bytes[bytes.len() - 2..], &[0xFF, 0xD9]);
        // 画像として読めること。
        let img = image::open(&output).unwrap();
        assert_eq!((img.width(), img.height()), (64, 48));
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn reads_empty_metadata() {
        let input = fixture_png();
        let meta = read_metadata_blocking(input.to_str().unwrap()).unwrap();
        assert_eq!(meta.format, "png");
        assert!(meta.png_text.is_empty());
        let jpg = fixture_jpeg();
        let meta = read_metadata_blocking(jpg.to_str().unwrap()).unwrap();
        assert_eq!(meta.format, "jpeg");
    }

    #[test]
    fn rejects_invalid_keyword() {
        let input = fixture_png();
        let output = input.with_file_name("bad.png");
        let entries = vec![PngTextEntry {
            keyword: String::new(),
            text: "x".to_string(),
        }];
        let err = save_metadata_with_format_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "png",
            85,
            &entries,
        )
        .unwrap_err();
        assert!(err.contains("キーワード"));
    }

    #[test]
    fn converts_format_while_saving() {
        let input = fixture_png();
        let output = input.with_file_name("converted.jpg");
        let entries = vec![PngTextEntry {
            keyword: "parameters".to_string(),
            text: "x".to_string(),
        }];
        let r = save_metadata_with_format_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "jpeg",
            85,
            &entries,
        )
        .unwrap();
        assert_eq!(r.format, "jpg");
        let img = image::open(&output).unwrap();
        assert_eq!((img.width(), img.height()), (64, 48));
        std::fs::remove_file(output).unwrap();
    }
}
