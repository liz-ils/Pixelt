mod convert;
mod mosaic;
mod resize;

use convert::{convert_batch, convert_image};
use mosaic::{apply_mosaic, load_preview};
use resize::resize_image;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            convert_image,
            convert_batch,
            apply_mosaic,
            load_preview,
            resize_image
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
