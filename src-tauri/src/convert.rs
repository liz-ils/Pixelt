//! 画像変換コマンド（デコード→再エンコード＋バッチ）。
//!
//! 入力: `image` クレートが読める形式（JPEG / PNG / WebP / GIF / BMP / TIFF / QOI 他）。
//! 出力: jpeg / png / webp / gif / bmp / tiff / qoi / avif。
//! 品質指定の意味:
//!
//!   - JPEG: エンコード品質に直接反映
//!   - WebP: 100 でロスレス、それ以外はロッシー品質
//!   - AVIF: ravif の品質指定（速度は固定）
//!   - PNG: 圧縮レベルに読み替え
//!
//! EXIF Orientation は読み込み時に正規化し、出力に EXIF は付与しない。

use std::io::Cursor;

use image::codecs::{
    bmp::BmpEncoder,
    gif::GifEncoder,
    jpeg::JpegEncoder,
    png::{CompressionType, FilterType, PngEncoder},
    qoi::QoiEncoder,
    tiff::TiffEncoder,
};
use image::{DynamicImage, ExtendedColorType, Frame, ImageEncoder};
use rayon::prelude::*;
use serde::Serialize;
use tauri::Emitter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Jpeg,
    Png,
    WebP,
    Gif,
    Bmp,
    Tiff,
    Qoi,
    Avif,
}

impl OutputFormat {
    fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Ok(Self::Jpeg),
            "png" => Ok(Self::Png),
            "webp" => Ok(Self::WebP),
            "gif" => Ok(Self::Gif),
            "bmp" => Ok(Self::Bmp),
            "tif" | "tiff" => Ok(Self::Tiff),
            "qoi" => Ok(Self::Qoi),
            "avif" => Ok(Self::Avif),
            other => Err(format!("未対応の出力形式です: {other}")),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::WebP => "webp",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
            Self::Qoi => "qoi",
            Self::Avif => "avif",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ConvertResult {
    pub output_path: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub input_bytes: u64,
    pub output_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct BatchError {
    pub input: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct BatchResult {
    pub output_dir: String,
    pub succeeded: Vec<ConvertResult>,
    pub failed: Vec<BatchError>,
}

/// EXIF Orientation を読み、表示通りの向きに回転・反転する。
fn normalize_orientation(img: DynamicImage, bytes: &[u8]) -> DynamicImage {
    let orientation = exif_orientation(bytes).unwrap_or(1);
    apply_orientation(img, orientation)
}

fn exif_orientation(bytes: &[u8]) -> Option<u32> {
    let exif = exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()?;
    let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    match &field.value {
        exif::Value::Short(v) => v.first().copied().map(u32::from),
        exif::Value::Long(v) => v.first().copied(),
        _ => None,
    }
}

fn apply_orientation(img: DynamicImage, orientation: u32) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

fn encode(img: &DynamicImage, format: OutputFormat, quality: u8) -> Result<Vec<u8>, String> {
    let (width, height) = (img.width(), img.height());
    let mut buf = Vec::new();
    match format {
        OutputFormat::Jpeg => {
            let rgb = img.to_rgb8();
            JpegEncoder::new_with_quality(&mut buf, quality)
                .write_image(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
                .map_err(|e| format!("JPEG エンコードに失敗しました: {e}"))?;
        }
        OutputFormat::Png => {
            let compression = if quality >= 95 {
                CompressionType::Best
            } else if quality >= 60 {
                CompressionType::Default
            } else {
                CompressionType::Fast
            };
            PngEncoder::new_with_quality(&mut buf, compression, FilterType::Sub)
                .write_image(
                    img.as_bytes(),
                    width,
                    height,
                    ExtendedColorType::from(img.color()),
                )
                .map_err(|e| format!("PNG エンコードに失敗しました: {e}"))?;
        }
        OutputFormat::WebP => {
            // 品質100はロスレス、それ以外はロッシー。
            if img.color().has_alpha() {
                let rgba = img.to_rgba8();
                let encoder = webp::Encoder::from_rgba(rgba.as_raw(), width, height);
                let mem = if quality >= 100 {
                    encoder.encode_lossless()
                } else {
                    encoder.encode(quality as f32)
                };
                buf.extend_from_slice(&mem);
            } else {
                let rgb = img.to_rgb8();
                let encoder = webp::Encoder::from_rgb(rgb.as_raw(), width, height);
                let mem = if quality >= 100 {
                    encoder.encode_lossless()
                } else {
                    encoder.encode(quality as f32)
                };
                buf.extend_from_slice(&mem);
            }
        }
        OutputFormat::Gif => {
            GifEncoder::new(&mut buf)
                .encode_frame(Frame::new(img.to_rgba8()))
                .map_err(|e| format!("GIF エンコードに失敗しました: {e}"))?;
        }
        OutputFormat::Bmp => {
            let rgb = img.to_rgb8();
            BmpEncoder::new(&mut buf)
                .write_image(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
                .map_err(|e| format!("BMP エンコードに失敗しました: {e}"))?;
        }
        OutputFormat::Tiff => {
            // TiffEncoder は Seek を要求するため Cursor 経由で書く。
            let mut cursor = Cursor::new(Vec::new());
            TiffEncoder::new(&mut cursor)
                .write_image(
                    img.as_bytes(),
                    width,
                    height,
                    ExtendedColorType::from(img.color()),
                )
                .map_err(|e| format!("TIFF エンコードに失敗しました: {e}"))?;
            buf = cursor.into_inner();
        }
        OutputFormat::Qoi => {
            if img.color().has_alpha() {
                let rgba = img.to_rgba8();
                QoiEncoder::new(&mut buf)
                    .write_image(rgba.as_raw(), width, height, ExtendedColorType::Rgba8)
                    .map_err(|e| format!("QOI エンコードに失敗しました: {e}"))?;
            } else {
                let rgb = img.to_rgb8();
                QoiEncoder::new(&mut buf)
                    .write_image(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
                    .map_err(|e| format!("QOI エンコードに失敗しました: {e}"))?;
            }
        }
        OutputFormat::Avif => {
            let threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            let encoder = ravif::Encoder::new()
                .with_quality(quality as f32)
                .with_speed(6)
                .with_num_threads(Some(threads));
            let encoded = if img.color().has_alpha() {
                let rgba = img.to_rgba8();
                let pixels: Vec<ravif::RGBA8> = rgba
                    .pixels()
                    .map(|p| ravif::RGBA8::new(p[0], p[1], p[2], p[3]))
                    .collect();
                encoder.encode_rgba(ravif::Img::new(
                    pixels.as_slice(),
                    width as usize,
                    height as usize,
                ))
            } else {
                let rgb = img.to_rgb8();
                let pixels: Vec<ravif::RGB8> = rgb
                    .pixels()
                    .map(|p| ravif::RGB8::new(p[0], p[1], p[2]))
                    .collect();
                encoder.encode_rgb(ravif::Img::new(
                    pixels.as_slice(),
                    width as usize,
                    height as usize,
                ))
            }
            .map_err(|e| format!("AVIF エンコードに失敗しました: {e}"))?;
            buf.extend_from_slice(&encoded.avif_file);
        }
    }
    Ok(buf)
}

fn convert_parsed(
    input: &str,
    output: &str,
    format: OutputFormat,
    quality: u8,
) -> Result<ConvertResult, String> {
    let bytes =
        std::fs::read(input).map_err(|e| format!("入力ファイルを開けません: {e}"))?;
    let img = image::load_from_memory(&bytes)
        .map_err(|e| format!("画像の読み込みに失敗しました: {e}"))?;
    let img = normalize_orientation(img, &bytes);
    let (width, height) = (img.width(), img.height());
    let encoded = encode(&img, format, quality)?;
    std::fs::write(output, &encoded)
        .map_err(|e| format!("出力ファイルの書き込みに失敗しました: {e}"))?;
    Ok(ConvertResult {
        output_path: output.to_string(),
        format: format.extension().to_string(),
        width,
        height,
        input_bytes: bytes.len() as u64,
        output_bytes: encoded.len() as u64,
    })
}

fn convert_blocking(
    input: &str,
    output: &str,
    format: &str,
    quality: u8,
) -> Result<ConvertResult, String> {
    convert_parsed(input, output, OutputFormat::parse(format)?, quality.clamp(1, 100))
}

fn convert_one(
    input: &str,
    output_dir: &str,
    format: OutputFormat,
    quality: u8,
    overwrite: bool,
) -> Result<ConvertResult, String> {
    let stem = std::path::Path::new(input)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("ファイル名を取得できません: {input}"))?;
    let output_path =
        std::path::Path::new(output_dir).join(format!("{stem}.{}", format.extension()));
    if output_path.exists() && !overwrite {
        return Err(format!(
            "上書き禁止のためスキップしました: {}",
            output_path.display()
        ));
    }
    convert_parsed(
        input,
        &output_path.to_string_lossy(),
        format,
        quality,
    )
}

/// 画像を変換する。重い処理なのでブロッキングプールに逃がす。
#[tauri::command]
pub async fn convert_image(
    input: String,
    output: String,
    format: String,
    quality: u8,
) -> Result<ConvertResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        convert_blocking(&input, &output, &format, quality)
    })
    .await
    .map_err(|e| format!("変換タスクが中断されました: {e}"))?
}

#[derive(Debug, Clone, Serialize)]
struct BatchProgress {
    done: usize,
    total: usize,
    current: String,
}

/// 複数画像を並列変換する。1件の失敗では全体を止めず、件ごとに結果を返す。
/// 進捗は `batch-progress` イベントで通知する。
#[tauri::command]
pub async fn convert_batch(
    app: tauri::AppHandle,
    inputs: Vec<String>,
    output_dir: String,
    format: String,
    quality: u8,
    overwrite: bool,
) -> Result<BatchResult, String> {
    let format = OutputFormat::parse(&format)?;
    let quality = quality.clamp(1, 100);
    if !std::path::Path::new(&output_dir).is_dir() {
        return Err(format!("出力ディレクトリが存在しません: {output_dir}"));
    }
    let result = tauri::async_runtime::spawn_blocking(move || {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let done = AtomicUsize::new(0);
        let total = inputs.len();
        let mut succeeded = Vec::with_capacity(inputs.len());
        let mut failed = Vec::new();
        let results: Vec<_> = inputs
            .par_iter()
            .map(|input| {
                let r = convert_one(input, &output_dir, format, quality, overwrite);
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = app.emit(
                    "batch-progress",
                    BatchProgress {
                        done: n,
                        total,
                        current: input.clone(),
                    },
                );
                r
            })
            .collect();
        for (input, result) in inputs.iter().zip(results) {
            match result {
                Ok(r) => succeeded.push(r),
                Err(e) => failed.push(BatchError {
                    input: input.clone(),
                    error: e,
                }),
            }
        }
        BatchResult {
            output_dir,
            succeeded,
            failed,
        }
    })
    .await
    .map_err(|e| format!("変換タスクが中断されました: {e}"))?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn fixture_png() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pixelt-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        let path = dir.join("input.png");
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn converts_png_to_jpeg() {
        let input = fixture_png();
        let output = input.with_extension("jpg");
        let r = convert_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "jpeg",
            80,
        )
        .unwrap();
        assert_eq!((r.width, r.height), (64, 48));
        assert!(r.output_bytes > 0);
        assert!(output.exists());
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn converts_png_to_png_with_quality_mapping() {
        let input = fixture_png();
        let output = input.with_file_name("output.png");
        let r = convert_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "png",
            30,
        )
        .unwrap();
        assert_eq!(r.format, "png");
        assert!(output.exists());
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn converts_png_to_webp_lossy() {
        let input = fixture_png();
        let output = input.with_extension("webp");
        let r = convert_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "webp",
            80,
        )
        .unwrap();
        assert_eq!((r.width, r.height), (64, 48));
        let back = image::open(&output).unwrap();
        assert_eq!((back.width(), back.height()), (64, 48));
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn converts_png_to_avif() {
        let input = fixture_png();
        let output = input.with_extension("avif");
        let r = convert_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "avif",
            60,
        )
        .unwrap();
        assert!(r.output_bytes > 0);
        let bytes = std::fs::read(&output).unwrap();
        assert!(bytes.len() > 8);
        assert_eq!(&bytes[4..8], b"ftyp");
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn converts_batch_in_parallel() {
        let input = fixture_png();
        let dir = input.parent().unwrap();
        let out_dir = dir.join("batch_out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let second = dir.join("second.png");
        std::fs::copy(&input, &second).unwrap();
        let results: Vec<_> = [input.clone(), second.clone()]
            .par_iter()
            .map(|p| {
                convert_one(
                    p.to_str().unwrap(),
                    out_dir.to_str().unwrap(),
                    OutputFormat::Jpeg,
                    80,
                    true,
                )
            })
            .collect();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.is_ok()));
        assert!(out_dir.join("input.jpg").exists());
        assert!(out_dir.join("second.jpg").exists());
        std::fs::remove_dir_all(out_dir).unwrap();
        std::fs::remove_file(second).unwrap();
    }

    #[test]
    fn batch_skips_existing_without_overwrite() {
        let input = fixture_png();
        let dir = input.parent().unwrap();
        let out_dir = dir.join("batch_skip");
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::copy(&input, out_dir.join("input.jpg")).unwrap();
        let err = convert_one(
            input.to_str().unwrap(),
            out_dir.to_str().unwrap(),
            OutputFormat::Jpeg,
            80,
            false,
        )
        .unwrap_err();
        assert!(err.contains("スキップ"));
        std::fs::remove_dir_all(out_dir).unwrap();
    }

    #[test]
    fn rejects_unknown_format() {
        let input = fixture_png();
        let err = convert_blocking(input.to_str().unwrap(), "unused", "xyz", 80).unwrap_err();
        assert!(err.contains("未対応"));
    }

    #[test]
    fn parses_format_aliases() {
        assert_eq!(OutputFormat::parse("JPG").unwrap(), OutputFormat::Jpeg);
        assert_eq!(OutputFormat::parse("tif").unwrap(), OutputFormat::Tiff);
        assert_eq!(OutputFormat::parse("avif").unwrap(), OutputFormat::Avif);
    }

    #[test]
    fn applies_exif_orientation_6() {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 0]));
        let rotated = apply_orientation(DynamicImage::ImageRgb8(img), 6);
        assert_eq!((rotated.width(), rotated.height()), (48, 64));
        let kept = apply_orientation(
            DynamicImage::ImageRgb8(ImageBuffer::from_pixel(4, 2, Rgb([0, 0, 0]))),
            1,
        );
        assert_eq!((kept.width(), kept.height()), (4, 2));
    }
}
