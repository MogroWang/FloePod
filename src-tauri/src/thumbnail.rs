use std::fs;

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db;
use crate::file_ops;
use crate::settings;
use crate::staging;
use crate::state::AppState;

const MAX_SIDE: u32 = 256;
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;
const MAX_ALLOCATION: u64 = 256 * 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailPayload {
    mime: String,
    bytes: Vec<u8>,
}

pub fn read(app: AppHandle, path: String) -> Result<Option<ThumbnailPayload>, String> {
    let state = app.state::<AppState>();
    // 与写操作串行取得源文件快照，耗时的格式探测和解码则在释放锁后执行。
    let bytes = {
        let _operation = state.file_ops.lock().unwrap();
        let (current, item) = {
            let connection = state.db.lock().unwrap();
            (
                staging::load_settings_from(&connection, &state)?,
                db::find_by_path(&connection, &path)?,
            )
        };
        let Some(item) = item else {
            return Ok(None);
        };
        settings::validate(&current, &staging::data_dir(&state))?;
        settings::validate_pod_for_io(&current, &staging::data_dir(&state), item.pod_id as u64)?;
        let target = staging::item_path(&item, &current)?;
        let extension = file_ops::extension(
            &target
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
        let Some(extension) = extension else {
            return Ok(None);
        };
        if !matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico"
        ) {
            return Ok(None);
        }
        let metadata = fs::metadata(&target).map_err(|error| error.to_string())?;
        if metadata.len() > MAX_SOURCE_BYTES {
            return Ok(None);
        }
        let bytes = fs::read(&target).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_SOURCE_BYTES {
            return Ok(None);
        }
        bytes
    };

    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_ALLOCATION);
    reader.limits(limits);
    let image = reader.decode().map_err(|error| error.to_string())?;
    let thumbnail = image.thumbnail(MAX_SIDE, MAX_SIDE);
    let mut png = Vec::new();
    image::DynamicImage::write_to(
        &thumbnail,
        std::io::Cursor::new(&mut png),
        image::ImageFormat::Png,
    )
    .map_err(|error| error.to_string())?;
    Ok(Some(ThumbnailPayload {
        mime: "image/png".into(),
        bytes: png,
    }))
}
