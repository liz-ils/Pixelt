//! リサイズコマンド。
//!
//! 幅・高さのいずれかに 0 を指定すると縦横比を維持して自動計算する。
//! 両方 0 の場合はエラー。フィルタは画質優先の Lanczos3 固定。

use image::imageops::FilterType;

use crate::convert::{ConvertResult, OutputFormat, encode, normalize_orientation};

#[allow(clippy::too_many_arguments)]
fn resize_blocking(
    input: &str,
    output: &str,
    format: &str,
    quality: u8,
    mode: &str,
    percent: u32,
    width: u32,
    height: u32,
    keep_aspect: bool,
) -> Result<ConvertResult, String> {
    let format = OutputFormat::parse(format)?;
    let quality = quality.clamp(1, 100);
    let bytes = std::fs::read(input).map_err(|e| format!("入力ファイルを開けません: {e}"))?;
    let img =
        image::load_from_memory(&bytes).map_err(|e| format!("画像の読み込みに失敗しました: {e}"))?;
    let img = normalize_orientation(img, &bytes);
    let (src_w, src_h) = (img.width(), img.height());
    let (dst_w, dst_h) = match mode.trim().to_ascii_lowercase().as_str() {
        "percent" => {
            let p = percent.clamp(1, 400) as u64;
            (
                ((src_w as u64 * p) / 100).max(1) as u32,
                ((src_h as u64 * p) / 100).max(1) as u32,
            )
        }
        "size" => {
            if width == 0 && height == 0 {
                return Err("幅・高さのどちらかを指定してください。".to_string());
            }
            if keep_aspect && width > 0 && height > 0 {
                let scale = f64::from(width.min(16384)) / f64::from(src_w).max(1.0);
                let scale = scale.min(f64::from(height.min(16384)) / f64::from(src_h).max(1.0));
                (
                    (f64::from(src_w) * scale).round().max(1.0) as u32,
                    (f64::from(src_h) * scale).round().max(1.0) as u32,
                )
            } else {
                match (width, height) {
                    (0, h) => (((src_w as u64 * h as u64) / src_h as u64).max(1) as u32, h),
                    (w, 0) => (w, ((src_h as u64 * w as u64) / src_w as u64).max(1) as u32),
                    (w, h) => (w, h),
                }
            }
        }
        other => return Err(format!("未対応のリサイズ方式です: {other}")),
    };
    let resized = img.resize_exact(dst_w, dst_h, FilterType::Lanczos3);
    let encoded = encode(&resized, format, quality)?;
    std::fs::write(output, &encoded)
        .map_err(|e| format!("出力ファイルの書き込みに失敗しました: {e}"))?;
    Ok(ConvertResult {
        output_path: output.to_string(),
        format: format.extension().to_string(),
        width: dst_w,
        height: dst_h,
        input_bytes: bytes.len() as u64,
        output_bytes: encoded.len() as u64,
    })
}

/// 画像を指定サイズにリサイズして保存する。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn resize_image(
    input: String,
    output: String,
    format: String,
    quality: u8,
    mode: String,
    percent: u32,
    width: u32,
    height: u32,
    keep_aspect: bool,
) -> Result<ConvertResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        resize_blocking(
            &input, &output, &format, quality, &mode, percent, width, height, keep_aspect,
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
        let dir = std::env::temp_dir().join(format!("pixelt-resize-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
        let path = dir.join(format!("input-{:?}.png", std::thread::current().id()));
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn resizes_to_exact_size() {
        let input = fixture_png();
        let output = input.with_extension("small.jpg");
        let r = resize_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "jpeg",
            85,
            "size",
            100,
            32,
            24,
            false,
        )
        .unwrap();
        assert_eq!((r.width, r.height), (32, 24));
        assert!(output.exists());
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn keeps_aspect_when_height_is_zero() {
        let input = fixture_png();
        let output = input.with_file_name("half.jpg");
        let r = resize_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "jpeg",
            85,
            "size",
            100,
            32,
            0,
            true,
        )
        .unwrap();
        assert_eq!((r.width, r.height), (32, 24));
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn fits_inside_box_with_aspect() {
        let input = fixture_png();
        let output = input.with_file_name("fit.jpg");
        let r = resize_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "jpeg",
            85,
            "size",
            100,
            32,
            32,
            true,
        )
        .unwrap();
        assert_eq!((r.width, r.height), (32, 24));
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn resizes_by_percent() {
        let input = fixture_png();
        let output = input.with_file_name("percent.jpg");
        let r = resize_blocking(
            input.to_str().unwrap(),
            output.to_str().unwrap(),
            "jpeg",
            85,
            "percent",
            50,
            0,
            0,
            true,
        )
        .unwrap();
        assert_eq!((r.width, r.height), (32, 24));
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn rejects_zero_size() {
        let input = fixture_png();
        let err = resize_blocking(
            input.to_str().unwrap(),
            "unused",
            "jpeg",
            85,
            "size",
            100,
            0,
            0,
            true,
        )
        .unwrap_err();
        assert!(err.contains("幅・高さ"));
    }

    #[test]
    fn rejects_unknown_mode() {
        let input = fixture_png();
        let err = resize_blocking(
            input.to_str().unwrap(),
            "unused",
            "jpeg",
            85,
            "stretch",
            100,
            32,
            32,
            false,
        )
        .unwrap_err();
        assert!(err.contains("未対応"));
    }
}
