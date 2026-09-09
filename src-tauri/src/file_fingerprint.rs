//! Streaming content identity for undo and recovery; links are never followed.
use sha2::{Digest, Sha256};
use std::{fs, io::Read, path::Path};

pub fn content(path: &Path) -> Result<String, String> {
    let mut hash = Sha256::new();
    walk(path, path, 0, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn walk(root: &Path, path: &Path, depth: usize, hash: &mut Sha256) -> Result<(), String> {
    if depth > 512 {
        return Err("目录嵌套过深，无法验证文件内容".into());
    }
    let before = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if crate::file_ops::is_reparse_or_symlink(&before) {
        return Err("不对符号链接或重解析点计算文件身份".into());
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|error| error.to_string())?
        .to_string_lossy();
    hash.update((relative.len() as u64).to_le_bytes());
    hash.update(relative.as_bytes());
    hash.update([u8::from(before.is_dir())]);
    if before.is_dir() {
        let mut entries = fs::read_dir(path)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        entries.sort_by_key(|entry| entry.file_name());
        hash.update((entries.len() as u64).to_le_bytes());
        for entry in entries {
            walk(root, &entry.path(), depth + 1, hash)?;
        }
    } else if before.is_file() {
        hash.update(before.len().to_le_bytes());
        let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
        let mut buffer = [0u8; 64 * 1024];
        let mut count = 0u64;
        loop {
            let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            count += read as u64;
            if count > before.len() {
                return Err("计算摘要时文件发生变化".into());
            }
            hash.update(&buffer[..read]);
        }
        if count != before.len() {
            return Err("计算摘要时文件发生变化".into());
        }
    } else {
        return Err("不支持的文件类型".into());
    }
    let after = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || before.created().ok() != after.created().ok()
        || before.file_type() != after.file_type()
    {
        return Err("计算摘要时文件发生变化".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_equal_length_edits_even_when_timestamp_is_restored() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("document.txt");
        fs::write(&path, b"aaaa").unwrap();
        let time = fs::metadata(&path).unwrap().modified().unwrap();
        let before = content(&path).unwrap();
        fs::write(&path, b"bbbb").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(time))
            .unwrap();
        assert_ne!(before, content(&path).unwrap());
    }
    #[test]
    fn copy_has_the_same_content_identity_but_renamed_children_do_not() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        let target = directory.path().join("target");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("one.txt"), b"payload").unwrap();
        crate::file_ops::copy_path(&source, &target).unwrap();
        assert_eq!(content(&source).unwrap(), content(&target).unwrap());
        crate::file_ops::rename_new(&target.join("one.txt"), &target.join("two.txt")).unwrap();
        assert_ne!(content(&source).unwrap(), content(&target).unwrap());
    }
}
