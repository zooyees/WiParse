//! Publish a completed, owned temporary file without replacing a destination.
//! Callers sync/close their writer and retain responsibility for temp cleanup.
use std::{io, path::Path};

#[cfg(windows)]
pub(crate) fn publish_new(temp: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    // canonicalize gives verbatim paths, including long local/UNC paths. The
    // destination itself must not be canonicalized: it must remain nonexistent.
    let source = temp.canonicalize()?;
    let parent = target.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = target.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "目标文件名无效"))?;
    let destination = parent.canonicalize()?.join(name);
    let wide = |path: &Path| -> io::Result<Vec<u16>> {
        let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) { return Err(io::Error::new(io::ErrorKind::InvalidInput, "文件路径包含空字符")); }
        value.push(0); Ok(value)
    };
    let source = wide(&source)?; let destination = wide(&destination)?;
    // Zero flags: no REPLACE_EXISTING and no COPY_ALLOWED. This is a same-volume
    // rename, not a hard link or a cross-volume copy/delete fallback.
    // SAFETY: both buffers are valid NUL-terminated UTF-16 for this synchronous call.
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn publish_new(temp: &Path, target: &Path) -> io::Result<()> {
    std::fs::hard_link(temp, target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publishes_unicode_names_and_never_replaces_existing_files() {
        let root=std::env::temp_dir().join(format!("wiparse_commit_{}",uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let temp=root.join("临时.txt");let target=root.join("测量记录.csv");
        std::fs::write(&temp,b"complete-data").unwrap();publish_new(&temp,&target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(),b"complete-data");
        let retry=root.join("another.tmp");std::fs::write(&retry,b"must-not-overwrite").unwrap();
        assert!(publish_new(&retry,&target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(),b"complete-data");assert!(retry.is_file());
        assert!(publish_new(&root.join("missing.tmp"),&root.join("missing.csv")).is_err());
        assert!(!root.join("missing.csv").exists());
        let _=std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn supports_long_paths_and_one_winner_for_concurrent_publication() {
        let root=std::env::temp_dir().join(format!("wiparse_long_commit_{}",uuid::Uuid::new_v4()));
        let directory=root.join("folder".repeat(20)).join("folder".repeat(20));
        std::fs::create_dir_all(&directory).unwrap();
        let target=directory.join("结果.txt");let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers:Vec<_>=(0..2).map(|index|{
            let temp=directory.join(format!("owned-{index}.tmp"));std::fs::write(&temp,format!("writer-{index}")).unwrap();
            let target=target.clone();let barrier=barrier.clone();
            std::thread::spawn(move||{barrier.wait();publish_new(&temp,&target).is_ok()})
        }).collect();
        let winners=workers.into_iter().filter_map(|worker|worker.join().ok()).filter(|ok|*ok).count();
        assert_eq!(winners,1);
        assert!([b"writer-0".to_vec(),b"writer-1".to_vec()].contains(&std::fs::read(&target).unwrap()));
        let _=std::fs::remove_dir_all(root);
    }
}
