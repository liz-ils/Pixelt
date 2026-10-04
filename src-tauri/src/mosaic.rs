//! モザイク適用コマンド。
//!
//! フロントの canvas エディタで指定された矩形領域にブロックモザイクをかける。
//! プレビュー用の縮小画像取得コマンドも提供する。

use image::{DynamicImage, ExtendedColorType, Rgba, RgbaImage};
use image::codecs::jpeg::JpegEncoder;
use image::ImageEncoder;
use serde::{Deserialize, Serialize};

use crate::convert::{ConvertResult, OutputFormat, encode, normalize_orientation};

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct MosaicRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Serialize)]
pub struct PreviewImage {
    pub data_url: String,
    pub width: u32,
    pub height: u32,
    pub full_width: u32,
    pub full_height: u32,
}

fn apply_mosaic_to_image(img: &mut RgbaImage, regions: &[MosaicRegion], pixel_size: u32) {
    let (w, h) = (img.width(), img.height());
    let px = pixel_size.clamp(2, 128);
    for r in regions {
        let x0 = r.x.min(w);
        let y0 = r.y.min(h);
        let x1 = r.x.saturating_add(r.width).min(w);
        let y1 = r.y.saturating_add(r.height).min(h);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let mut y = y0;
        while y < y1 {
            let by1 = y.saturating_add(px).min(y1);
            let mut x = x0;
            while x < x1 {
                let bx1 = x.saturating_add(px).min(x1);
                let (mut sr, mut sg, mut sb, mut sa, mut n) =
                    (0u64, 0u64, 0u64, 0u64, 0u64);
                for py in y..by1 {
                    for bx in x..bx1 {
                        let p = img.get_pixel(bx, py);
                        sr += u64::from(p[0]);
                        sg += u64::from(p[1]);
                        sb += u64::from(p[2]);
                        sa += u64::from(p[3]);
                        n += 1;
                    }
                }
                if n == 0 {
                    continue;
                }
                let avg = Rgba([
                    (sr / n) as u8,
                    (sg / n) as u8,
                    (sb / n) as u8,
                    (sa / n) as u8,
                ]);
                for py in y..by1 {
                    for bx in x..bx1 {
                        img.put_pixel(bx, py, avg);
                    }
                }
                x = bx1;
            }
            y = by1;
        }
    }
}

fn apply_mosaic_blocking(
    input: &str,
    output: &str,
    format: &str,
    quality: u8,
    regions: &[MosaicRegion],
    pixel_size: u32,
) -> Result<ConvertResult, String> {
    let format = OutputFormat::parse(format)?;
    let quality = quality.clamp(1, 100);
    let bytes = std::fs::read(input).map_err(|e| format!("入力ファイルを開けません: {e}"))?;
    let img =
        image::load_from_memory(&bytes).map_err(|e| format!("画像の読み込みに失敗しました: {e}"))?;
    let img = normalize_orientation(img, &bytes);
    let (width, height) = (img.width(), img.height());
    let mut rgba = img.to_rgba8();
    apply_mosaic_to_image(&mut rgba, regions, pixel_size);
    let encoded = encode(&DynamicImage::ImageRgba8(rgba), format, quality)?;
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

fn load_preview_blocking(input: &str, max_size: u32) -> Result<PreviewImage, String> {
    let bytes = std::fs::read(input).map_err(|e| format!("入力ファイルを開けません: {e}"))?;
    let img =
        image::load_from_memory(&bytes).map_err(|e| format!("画像の読み込みに失敗しました: {e}"))?;
    let img = normalize_orientation(img, &bytes);
    let max = max_size.clamp(64, 1024);
    let thumb = img.thumbnail(max, max);
    let (width, height) = (thumb.width(), thumb.height());
    let (full_width, full_height) = (img.width(), img.height());
    let rgb = thumb.to_rgb8();
    let mut buf = Vec::new();
    JpegEncoder::new_with_quality(&mut buf, 75)
        .write_image(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
        .map_err(|e| format!("プレビューの作成に失敗しました: {e}"))?;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    Ok(PreviewImage {
        data_url: format!("data:image/jpeg;base64,{}", STANDARD.encode(&buf)),
        width,
        height,
        full_width,
        full_height,
    })
}

/// 指定領域にモザイクをかけて保存する。
#[tauri::command]
pub async fn apply_mosaic(
    input: String,
    output: String,
    format: String,
    quality: u8,
    regions: Vec<MosaicRegion>,
    pixel_size: u32,
) -> Result<ConvertResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        apply_mosaic_blocking(&input, &output, &format, quality, &regions, pixel_size)
    })
    .await
    .map_err(|e| format!("変換タスクが中断されました: {e}"))?
}

/// 編集用の縮小プレビュー画像を data URL で返す。
#[tauri::command]
pub async fn load_preview(input: String, max_size: u32) -> Result<PreviewImage, String> {
    tauri::async_runtime::spawn_blocking(move || load_preview_blocking(&input, max_size))
        .await
        .map_err(|e| format!("変換タスクが中断されました: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn fixture_png() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pixelt-mosaic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        let path = dir.join("input.png");
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn applies_mosaic_to_region() {
        let input = fixture_png();
        let output = input.with_extension("mosaic.jpg");
        let regions = vec![MosaicRegion {
            x: 0,
            y: 0,
            width: 64,
            height: 48,
        }];
        let r = apply_mosaic_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "jpeg",
            90,
            &regions,
            8,
        )
        .unwrap();
        assert_eq!((r.width, r.height), (64, 48));
        assert!(output.exists());
        // ブロック内が平均色で均一化されていること。
        let back = image::open(&output).unwrap().to_rgb8();
        assert_eq!(back.get_pixel(0, 0), back.get_pixel(7, 7));
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn ignores_out_of_bounds_region() {
        let input = fixture_png();
        let output = input.with_file_name("oob.jpg");
        let regions = vec![MosaicRegion {
            x: 1000,
            y: 1000,
            width: 10,
            height: 10,
        }];
        let r = apply_mosaic_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "jpeg",
            90,
            &regions,
            8,
        )
        .unwrap();
        assert_eq!((r.width, r.height), (64, 48));
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn loads_preview_as_data_url() {
        let input = fixture_png();
        let p = load_preview_blocking(input.to_str().unwrap(), 256).unwrap();
        assert!(p.data_url.starts_with("data:image/jpeg;base64,"));
        assert!(p.width <= 256 && p.height <= 256);
    }
}
