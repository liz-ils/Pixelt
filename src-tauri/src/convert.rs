//! 画像変換コマンド（デコード→再エンコード）。
//!
//! 入力: `image` クレートが読める形式（JPEG / PNG / WebP / GIF / BMP / TIFF / QOI 他）。
//! 出力: jpeg / png / webp / gif / bmp / tiff / qoi。
//! 品質指定は JPEG のエンコード品質に直接反映し、PNG は圧縮レベルに読み替える。
//! WebP は `image` 0.25 のエンコーダーがロスレス専用のため品質指定を受け付けない。

use std::io::Cursor;

use image::codecs::{
    bmp::BmpEncoder,
    gif::GifEncoder,
    jpeg::JpegEncoder,
    png::{CompressionType, FilterType, PngEncoder},
    qoi::QoiEncoder,
    tiff::TiffEncoder,
    webp::WebPEncoder,
};
use image::{DynamicImage, ExtendedColorType, Frame, ImageEncoder};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Jpeg,
    Png,
    WebP,
    Gif,
    Bmp,
    Tiff,
    Qoi,
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
            // ロスレス専用。8bit バッファは借用し、それ以外は RGBA8 に寄せる。
            let converted;
            let (pixels, color): (&[u8], ExtendedColorType) = match img {
                DynamicImage::ImageRgb8(b) => (b.as_raw(), ExtendedColorType::Rgb8),
                DynamicImage::ImageRgba8(b) => (b.as_raw(), ExtendedColorType::Rgba8),
                DynamicImage::ImageLuma8(b) => (b.as_raw(), ExtendedColorType::L8),
                DynamicImage::ImageLumaA8(b) => (b.as_raw(), ExtendedColorType::La8),
                _ => {
                    converted = img.to_rgba8();
                    (converted.as_raw(), ExtendedColorType::Rgba8)
                }
            };
            WebPEncoder::new_lossless(&mut buf)
                .encode(pixels, width, height, color)
                .map_err(|e| format!("WebP エンコードに失敗しました: {e}"))?;
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
    }
    Ok(buf)
}

fn convert_blocking(
    input: &str,
    output: &str,
    format: &str,
    quality: u8,
) -> Result<ConvertResult, String> {
    let format = OutputFormat::parse(format)?;
    let quality = quality.clamp(1, 100);
    let input_bytes = std::fs::metadata(input)
        .map_err(|e| format!("入力ファイルを開けません: {e}"))?
        .len();
    let img =
        image::open(input).map_err(|e| format!("画像の読み込みに失敗しました: {e}"))?;
    let (width, height) = (img.width(), img.height());
    let encoded = encode(&img, format, quality)?;
    std::fs::write(output, &encoded)
        .map_err(|e| format!("出力ファイルの書き込みに失敗しました: {e}"))?;
    Ok(ConvertResult {
        output_path: output.to_string(),
        format: format.extension().to_string(),
        width,
        height,
        input_bytes,
        output_bytes: encoded.len() as u64,
    })
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
    fn rejects_unknown_format() {
        let input = fixture_png();
        let err = convert_blocking(input.to_str().unwrap(), "unused", "xyz", 80).unwrap_err();
        assert!(err.contains("未対応"));
    }

    #[test]
    fn parses_format_aliases() {
        assert_eq!(OutputFormat::parse("JPG").unwrap(), OutputFormat::Jpeg);
        assert_eq!(OutputFormat::parse("tif").unwrap(), OutputFormat::Tiff);
    }
}
