//! 持久化操作时间线、补偿动作与 24 小时基础撤销。
//!
//! 文件操作先完成自身的原子提交，再把可逆步骤写入 operations / operation_items /
//! compensations。历史写入失败不能反向破坏已经成功的文件操作，因此调用方应记录
//! 日志并把操作结果照常返回；撤销则始终保守校验文件身份，内容已变化时拒绝删除。

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::file_ops;

pub fn signature(path: &Path) -> Result<String, String> {
    Ok(format!(
        "sha256:{}:{}",
        crate::file_fingerprint::content(path)?,
        legacy_signature(path)?
    ))
}

fn legacy_signature(path: &Path) -> Result<String, String> {
    let mut hash = 0xcbf29ce484222325u64;
    signature_path(path, path, &mut hash)?;
    Ok(format!("{hash:016x}"))
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= *byte as u64;
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}

fn modified_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn signature_path(root: &Path, path: &Path, hash: &mut u64) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("无法读取 {}: {error}", path.display()))?;
    if file_ops::is_reparse_or_symlink(&metadata) {
        return Err(format!(
            "不对符号链接或目录重解析点执行撤销: {}",
            path.display()
        ));
    }
    let relative = path.strip_prefix(root).unwrap_or(path).to_string_lossy();
    hash_bytes(hash, relative.as_bytes());
    hash_bytes(hash, &metadata.len().to_le_bytes());
    hash_bytes(hash, &modified_ms(&metadata).to_le_bytes());
    hash_bytes(hash, &[metadata.is_dir() as u8]);
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)
            .map_err(|error| format!("无法读取目录 {}: {error}", path.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("无法枚举目录 {}: {error}", path.display()))?;
        children.sort_by_key(|entry| entry.file_name().to_string_lossy().to_lowercase());
        for child in children {
            signature_path(root, &child.path(), hash)?;
        }
    }
    Ok(())
}

pub(super) fn verify_signature(path: &Path, expected: Option<&str>) -> Result<(), String> {
    let Some(expected) = expected else {
        return Err("操作记录缺少文件身份，为避免误删已拒绝撤销".into());
    };
    // Historical records retain their original interpretation; new records also verify every byte.
    let actual = if expected.starts_with("sha256:") {
        signature(path)?
    } else {
        legacy_signature(path)?
    };
    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "文件在操作后已经变化，为避免误删已拒绝撤销: {}",
            path.display()
        ))
    }
}
