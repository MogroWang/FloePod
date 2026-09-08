//! Clean only a temporary copy; publication and original-file safety belong to the caller.
use super::metadata;
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

pub(crate) fn clean_image(source: &Path, output: &Path) -> Result<Vec<String>, String> {
    let mut reader = image::ImageReader::open(source).map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("无法在安全资源限制内解码图片: {error}"))?;
    let format = image::ImageFormat::from_path(source).unwrap_or(image::ImageFormat::Png);
    image
        .save_with_format(output, format)
        .map_err(|error| format!("无法写入清理后的图片: {error}"))?;
    Ok(vec!["EXIF、GPS、作者、设备和软件元数据".into()])
}

pub(crate) fn clean_pdf(source: &Path, output: &Path) -> Result<Vec<String>, String> {
    let input = fs::File::open(source).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    input
        .take(metadata::MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > metadata::MAX_INPUT_BYTES {
        return Err("PDF 超过安全清理大小限制".into());
    }
    let mut document = lopdf::Document::load_mem_with_options(
        &bytes,
        lopdf::LoadOptions {
            strict: true,
            max_decompressed_size: Some(metadata::MAX_PROPERTY_BYTES as usize),
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())?;
    if document.is_encrypted() {
        return Err("受密码保护的 PDF 无法清理".into());
    }
    let mut removed = Vec::new();
    if document.trailer.remove(b"Info").is_some() {
        removed.push("PDF 文档属性".into());
    }
    let root = document
        .trailer
        .get(b"Root")
        .and_then(lopdf::Object::as_reference)
        .ok();
    if let Some(root) = root {
        if let Ok(catalog) = document.get_dictionary_mut(root) {
            if catalog.remove(b"Metadata").is_some() {
                removed.push("PDF XMP 元数据".into());
            }
        }
    }
    document.save(output).map_err(|error| error.to_string())?;
    Ok(removed)
}

pub(super) fn scrub_xml(xml: &str) -> Result<(String, Vec<String>), String> {
    metadata::xml_properties(xml.as_bytes())
        .map_err(|error| format!("文档属性无法安全解析: {error:?}"))?;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut writer = quick_xml::Writer::new(Vec::new());
    let mut skipped_depth = 0usize;
    let mut removed = Vec::new();
    loop {
        let event = reader.read_event().map_err(|error| error.to_string())?;
        match &event {
            quick_xml::events::Event::Eof => break,
            quick_xml::events::Event::Start(element) => {
                if skipped_depth > 0 {
                    skipped_depth += 1;
                    continue;
                }
                if let Some(label) = metadata::property_label(element.local_name().as_ref()) {
                    removed.push(label.into());
                    skipped_depth = 1;
                    continue;
                }
            }
            quick_xml::events::Event::Empty(element) if skipped_depth == 0 => {
                if let Some(label) = metadata::property_label(element.local_name().as_ref()) {
                    removed.push(label.into());
                    continue;
                }
            }
            quick_xml::events::Event::End(_) if skipped_depth > 0 => {
                skipped_depth -= 1;
                continue;
            }
            _ => {}
        }
        if skipped_depth == 0 {
            writer
                .write_event(event)
                .map_err(|error| error.to_string())?;
        }
    }
    Ok((
        String::from_utf8(writer.into_inner()).map_err(|error| error.to_string())?,
        removed,
    ))
}

pub(crate) fn clean_office(source: &Path, output: &Path) -> Result<Vec<String>, String> {
    let input = fs::File::open(source).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(input).map_err(|error| error.to_string())?;
    if archive.len() > 10_000 {
        return Err("文档压缩条目过多，拒绝清理".into());
    }
    let output_file = fs::File::create(output).map_err(|error| error.to_string())?;
    let mut writer = ZipWriter::new(output_file);
    let mut removed = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let name = entry.name().replace('\\', "/");
        let options = SimpleFileOptions::default().compression_method(entry.compression());
        if entry.is_dir() {
            writer
                .add_directory(name, options)
                .map_err(|error| error.to_string())?;
            continue;
        }
        if matches!(
            name.as_str(),
            "docProps/core.xml" | "docProps/app.xml" | "meta.xml"
        ) {
            if entry.size() > 2 * 1024 * 1024 {
                return Err(format!("文档属性文件异常过大，拒绝清理: {name}"));
            }
            let mut xml = String::new();
            entry
                .by_ref()
                .take(metadata::MAX_PROPERTY_BYTES + 1)
                .read_to_string(&mut xml)
                .map_err(|error| error.to_string())?;
            if xml.len() as u64 > metadata::MAX_PROPERTY_BYTES {
                return Err("文档属性解压内容超过限制".into());
            }
            let (xml, fields) = scrub_xml(&xml)?;
            writer
                .start_file(name, options)
                .map_err(|error| error.to_string())?;
            removed.extend(fields);
            writer
                .write_all(xml.as_bytes())
                .map_err(|error| error.to_string())?;
        } else {
            writer
                .raw_copy_file(entry)
                .map_err(|error| error.to_string())?;
        }
    }
    writer.finish().map_err(|error| error.to_string())?;
    removed.sort();
    removed.dedup();
    Ok(removed)
}
