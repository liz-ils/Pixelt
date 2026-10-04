//! 透かし適用コマンド。
//!
//! テキスト透かし（フォントファイル指定）と画像透かし（ロゴ重ね）に対応。
//! 位置は9分割指定、透明度・余白・サイズを調整できる。

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use image::{DynamicImage, Rgba, RgbaImage};
use serde::Deserialize;

use crate::convert::{ConvertResult, OutputFormat, encode, normalize_orientation};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WatermarkSpec {
    Text {
        text: String,
        font_path: String,
        size: f32,
        color: [u8; 3],
    },
    Image {
        path: String,
        scale: f32,
    },
}

#[derive(Debug, Clone, Copy)]
enum Position {
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    Center,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Position {
    fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "top-left" => Ok(Self::TopLeft),
            "top-center" => Ok(Self::TopCenter),
            "top-right" => Ok(Self::TopRight),
            "middle-left" => Ok(Self::MiddleLeft),
            "center" => Ok(Self::Center),
            "middle-right" => Ok(Self::MiddleRight),
            "bottom-left" => Ok(Self::BottomLeft),
            "bottom-center" => Ok(Self::BottomCenter),
            "bottom-right" => Ok(Self::BottomRight),
            other => Err(format!("未対応の位置指定です: {other}")),
        }
    }

    fn origin(self, base_w: u32, base_h: u32, ov_w: u32, ov_h: u32, margin: u32) -> (u32, u32) {
        let x = match self {
            Self::TopLeft | Self::MiddleLeft | Self::BottomLeft => margin,
            Self::TopCenter | Self::Center | Self::BottomCenter => {
                base_w.saturating_sub(ov_w) / 2
            }
            Self::TopRight | Self::MiddleRight | Self::BottomRight => {
                base_w.saturating_sub(ov_w).saturating_sub(margin)
            }
        };
        let y = match self {
            Self::TopLeft | Self::TopCenter | Self::TopRight => margin,
            Self::MiddleLeft | Self::Center | Self::MiddleRight => {
                base_h.saturating_sub(ov_h) / 2
            }
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight => {
                base_h.saturating_sub(ov_h).saturating_sub(margin)
            }
        };
        (x.min(base_w), y.min(base_h))
    }
}

fn render_text_layer(text: &str, font_bytes: &[u8], size: f32, color: [u8; 3]) -> Result<RgbaImage, String> {
    let font = FontRef::try_from_slice(font_bytes)
        .map_err(|_| "フォントファイルを読み込めません（TTF/OTF を指定してください）。".to_string())?;
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let mut caret = point(0.0, scaled.ascent());
    let mut positioned = Vec::new();
    for ch in text.chars() {
        let mut glyph = scaled.scaled_glyph(ch);
        glyph.position = caret;
        caret.x += scaled.h_advance(glyph.id);
        positioned.push(glyph);
    }
    let width = caret.x.ceil().max(1.0) as u32;
    let height = scaled.height().ceil().max(1.0) as u32;
    let mut layer = RgbaImage::new(width, height);
    for glyph in positioned {
        if let Some(outlined) = font.outline_glyph(glyph) {
            outlined.draw(|x, y, coverage| {
                if x < width && y < height {
                    layer.put_pixel(
                        x,
                        y,
                        Rgba([color[0], color[1], color[2], (coverage * 255.0) as u8]),
                    );
                }
            });
        }
    }
    Ok(layer)
}

/// オーバーレイ画像を透明度付きで合成する。
fn overlay(base: &mut RgbaImage, layer: &RgbaImage, x0: u32, y0: u32, opacity: f32) {
    let (bw, bh) = (base.width(), base.height());
    for (lx, ly, p) in layer.enumerate_pixels() {
        let (bx, by) = (x0 + lx, y0 + ly);
        if bx >= bw || by >= bh {
            continue;
        }
        let alpha = (f32::from(p[3]) / 255.0) * opacity;
        if alpha <= 0.0 {
            continue;
        }
        let dst = base.get_pixel(bx, by);
        let mix = |src: u8, d: u8| (f32::from(src) * alpha + f32::from(d) * (1.0 - alpha)) as u8;
        base.put_pixel(bx, by, Rgba([mix(p[0], dst[0]), mix(p[1], dst[1]), mix(p[2], dst[2]), 255]));
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_watermark_blocking(
    input: &str,
    output: &str,
    format: &str,
    quality: u8,
    watermark: &WatermarkSpec,
    position: &str,
    margin: u32,
    opacity: f32,
) -> Result<ConvertResult, String> {
    let format = OutputFormat::parse(format)?;
    let quality = quality.clamp(1, 100);
    let position = Position::parse(position)?;
    let margin = margin.min(512);
    let opacity = opacity.clamp(0.01, 1.0);
    let bytes = std::fs::read(input).map_err(|e| format!("入力ファイルを開けません: {e}"))?;
    let img =
        image::load_from_memory(&bytes).map_err(|e| format!("画像の読み込みに失敗しました: {e}"))?;
    let img = normalize_orientation(img, &bytes);
    let (base_w, base_h) = (img.width(), img.height());
    let mut base = img.to_rgba8();

    let layer = match watermark {
        WatermarkSpec::Text {
            text,
            font_path,
            size,
            color,
        } => {
            if text.trim().is_empty() {
                return Err("透かし文字が空です。".to_string());
            }
            let font_bytes = std::fs::read(font_path)
                .map_err(|e| format!("フォントファイルを開けません: {e}"))?;
            render_text_layer(text, &font_bytes, size.clamp(8.0, 512.0), *color)?
        }
        WatermarkSpec::Image { path, scale } => {
            let ov_bytes =
                std::fs::read(path).map_err(|e| format!("透かし画像を開けません: {e}"))?;
            let ov = image::load_from_memory(&ov_bytes)
                .map_err(|e| format!("透かし画像の読み込みに失敗しました: {e}"))?;
            let target_w = ((base_w as f32 * scale.clamp(0.01, 1.0)) as u32).max(1);
            let h = ((ov.height() as f32 * target_w as f32) / ov.width() as f32) as u32;
            ov.resize_exact(target_w, h.max(1), image::imageops::FilterType::Lanczos3)
                .to_rgba8()
        }
    };
    let (ox, oy) = position.origin(base_w, base_h, layer.width(), layer.height(), margin);
    overlay(&mut base, &layer, ox, oy, opacity);

    let encoded = encode(&DynamicImage::ImageRgba8(base), format, quality)?;
    std::fs::write(output, &encoded)
        .map_err(|e| format!("出力ファイルの書き込みに失敗しました: {e}"))?;
    Ok(ConvertResult {
        output_path: output.to_string(),
        format: format.extension().to_string(),
        width: base_w,
        height: base_h,
        input_bytes: bytes.len() as u64,
        output_bytes: encoded.len() as u64,
    })
}

/// 透かしを適用して保存する。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn apply_watermark(
    input: String,
    output: String,
    format: String,
    quality: u8,
    watermark: WatermarkSpec,
    position: String,
    margin: u32,
    opacity: f32,
) -> Result<ConvertResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        apply_watermark_blocking(
            &input, &output, &format, quality, &watermark, &position, margin, opacity,
        )
    })
    .await
    .map_err(|e| format!("変換タスクが中断されました: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn fixture_png() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pixelt-watermark-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(128, 96, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 200]));
        let path = dir.join("input.png");
        img.save(&path).unwrap();
        path
    }

    fn system_font() -> Option<std::path::PathBuf> {
        for name in ["arial.ttf", "meiryo.ttf", "msgothic.ttc", "YuGothM.ttc"] {
            let p = std::path::Path::new(r"C:\Windows\Fonts").join(name);
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    #[test]
    fn applies_image_watermark() {
        let input = fixture_png();
        let logo = input.with_file_name("logo.png");
        let logo_img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(16, 16, Rgb([255, 0, 0]));
        logo_img.save(&logo).unwrap();
        let output = input.with_extension("wm.jpg");
        let spec = WatermarkSpec::Image {
            path: logo.to_string_lossy().into_owned(),
            scale: 0.25,
        };
        let r = apply_watermark_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "jpeg",
            85,
            &spec,
            "bottom-right",
            8,
            0.8,
        )
        .unwrap();
        assert_eq!((r.width, r.height), (128, 96));
        assert!(output.exists());
        std::fs::remove_file(output).unwrap();
        std::fs::remove_file(logo).unwrap();
    }

    #[test]
    fn applies_text_watermark_with_system_font() {
        let Some(font) = system_font() else {
            eprintln!("システムフォントが無いためスキップ");
            return;
        };
        if font.extension().and_then(|e| e.to_str()) == Some("ttc") {
            eprintln!("TTC のためスキップ");
            return;
        }
        let input = fixture_png();
        let output = input.with_file_name("text.jpg");
        let spec = WatermarkSpec::Text {
            text: "Pixelt".to_string(),
            font_path: font.to_string_lossy().into_owned(),
            size: 24.0,
            color: [255, 255, 255],
        };
        let r = apply_watermark_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "jpeg",
            85,
            &spec,
            "bottom-right",
            8,
            0.9,
        )
        .unwrap();
        assert_eq!((r.width, r.height), (128, 96));
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn rejects_missing_font() {
        let input = fixture_png();
        let spec = WatermarkSpec::Text {
            text: "x".to_string(),
            font_path: r"C:\nonexistent\font.ttf".to_string(),
            size: 24.0,
            color: [0, 0, 0],
        };
        let err = apply_watermark_blocking(
            input.to_str().unwrap(),
            "unused",
            "jpeg",
            85,
            &spec,
            "center",
            0,
            1.0,
        )
        .unwrap_err();
        assert!(err.contains("フォント"));
    }

    #[test]
    fn rejects_unknown_position() {
        assert!(Position::parse("middle-earth").is_err());
        assert_eq!(
            Position::parse("top-left").unwrap().origin(100, 100, 10, 10, 5),
            (5, 5)
        );
    }
}
