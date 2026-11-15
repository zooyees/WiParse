//! Windows project-relative, no-follow, exclusive AI evidence output.
//! Every path component is opened relative to a held directory handle; the
//! handles deny delete sharing until writing finishes. No model path is used.
use std::{fs::File, io::Write, path::{Path, PathBuf}};

pub(super) struct Destination {
    #[cfg(windows)]
    directories: Vec<File>,
    display_dir: PathBuf,
}

impl Destination {
    pub(super) fn prepare(root: &Path, project: &str) -> Result<Self, String> {
        if !wiparse_core::project::is_valid_id(project) {
            return Err("AI.PROJECT_INVALID".into());
        }
        #[cfg(windows)] {
            let root = root.canonicalize().map_err(|e| format!("AI.EVIDENCE_ROOT_UNAVAILABLE: {e}"))?;
            let mut root_handle = windows::open_root(&root, false)?;
            let mut projects = match windows::open_directory(&root_handle, "projects", false, false, false) {
                Ok(handle) => handle,
                Err(_) => {
                    root_handle = windows::open_root(&root, true)?;
                    windows::open_directory(&root_handle, "projects", true, false, false)?
                }
            };
            let mut project_handle = match windows::open_directory(&projects, project, false, false, false) {
                Ok(handle) => handle,
                Err(_) => {
                    projects = windows::open_directory(&root_handle, "projects", false, true, false)?;
                    windows::open_directory(&projects, project, true, false, false)?
                }
            };
            let evidence = match windows::open_directory(&project_handle, "ai-evidence", false, false, true) {
                Ok(handle) => handle,
                Err(_) => {
                    project_handle = windows::open_directory(&projects, project, false, true, false)?;
                    windows::open_directory(&project_handle, "ai-evidence", true, false, true)?
                }
            };
            Ok(Self { directories: vec![root_handle, projects, project_handle, evidence],
                display_dir: root.join("projects").join(project).join("ai-evidence") })
        }
        #[cfg(not(windows))] {
            let _ = root;
            Err("AI.SECURE_ARCHIVE_UNSUPPORTED: use the manual Save As path on this platform".into())
        }
    }

    pub(super) fn write_new(&self, bytes: &[u8], extension: &str, cancel: Option<&std::sync::atomic::AtomicBool>) -> Result<PathBuf, String> {
        #[cfg(windows)] {
            if extension.is_empty() || extension.len() > 8 || !extension.bytes().all(|b| b.is_ascii_alphanumeric()) {
                return Err("AI.EVIDENCE_INVALID_EXTENSION".into());
            }
            if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
                return Err("AI.CANCELLED_BEFORE_WRITE".into());
            }
            let name = format!("ai-wave-{}.{extension}", uuid::Uuid::new_v4());
            let partial = format!(".{name}.partial");
            let partial_path = self.display_dir.join(&partial);
            let directory = self.directories.last().ok_or("AI.EVIDENCE_DIRECTORY_UNAVAILABLE")?;
            let mut file = windows::create_file(directory, &partial)?;
            file.write_all(bytes).map_err(|e| format!("AI.EVIDENCE_WRITE_FAILED: {e}; partial file may remain at {}", partial_path.display()))?;
            file.sync_all().map_err(|e| format!("AI.EVIDENCE_SYNC_FAILED: {e}; partial file may remain at {}", partial_path.display()))?;
            if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
                return Err(format!("AI.CANCELLED_BEFORE_COMMIT: partial file may remain at {}", partial_path.display()));
            }
            windows::rename_file(&file, directory, &name)
                .map_err(|e| format!("{e}; partial path: {}", partial_path.display()))?;
            let path = self.display_dir.join(name);
            if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire)) {
                return Err(format!("AI.CANCELLED_AFTER_COMMIT: file may have been saved at {}; verify before retry", path.display()));
            }
            let committed = same_file::Handle::from_file(file.try_clone().map_err(|e| e.to_string())?)
                .map_err(|e| format!("AI.EVIDENCE_IDENTITY_UNAVAILABLE: {e}; committed file may exist"))?;
            let visible = windows::path_identity(&path)
                .map_err(|e| format!("AI.EVIDENCE_PATH_CHANGED: {e}; committed file may exist"))?;
            if committed != visible {
                return Err("AI.EVIDENCE_PATH_CHANGED: committed file may exist".into());
            }
            Ok(path)
        }
        #[cfg(not(windows))] {
            let _ = (bytes, extension, cancel);
            Err("AI.SECURE_ARCHIVE_UNSUPPORTED".into())
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, fs::OpenOptions, os::windows::{fs::{MetadataExt, OpenOptionsExt}, io::{AsRawHandle, FromRawHandle}}};

    type Handle = *mut c_void;
    #[repr(C)] struct UnicodeString { length: u16, maximum_length: u16, buffer: *mut u16 }
    #[repr(C)] struct ObjectAttributes { length: u32, root: Handle, name: *mut UnicodeString,
        attributes: u32, security_descriptor: *mut c_void, security_qos: *mut c_void }
    #[repr(C)] struct IoStatus { status_or_pointer: isize, information: usize }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtCreateFile(handle: *mut Handle, access: u32, attributes: *mut ObjectAttributes,
            io: *mut IoStatus, allocation: *mut i64, file_attributes: u32, share: u32,
            disposition: u32, options: u32, ea: *mut c_void, ea_length: u32) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
        fn NtSetInformationFile(handle: Handle, io: *mut IoStatus, information: *mut c_void,
            length: u32, information_class: u32) -> i32;
    }
    const REPARSE: u32 = 0x400;
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2; // Deliberately omit FILE_SHARE_DELETE.
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const READ_ATTRIBUTES: u32 = 0x80;
    const OPEN_REPARSE: u32 = 0x0020_0000;
    const SYNC_NONALERT: u32 = 0x20;
    const DIRECTORY_FILE: u32 = 0x1;
    const NON_DIRECTORY_FILE: u32 = 0x40;

    pub(super) fn open_root(path: &Path, add_directory: bool) -> Result<File, String> {
        // Request creation access only when a missing child needs it.
        let access = SYNCHRONIZE | READ_ATTRIBUTES | 0x1 | 0x20 | if add_directory {0x4} else {0};
        let handle = OpenOptions::new().read(true).access_mode(access).share_mode(SHARE_READ_WRITE)
            .custom_flags(0x0200_0000 | OPEN_REPARSE).open(path)
            .map_err(|e| format!("AI.EVIDENCE_ROOT_OPEN_FAILED: {e}"))?;
        check_directory(&handle)?;
        // Compare volume/file identity instead of path spelling (drive case,
        // volume aliases and normalization need not have identical strings).
        let expected = path_identity(path).map_err(|e| format!("AI.EVIDENCE_ROOT_IDENTITY_UNAVAILABLE: {e}"))?;
        let actual = same_file::Handle::from_file(handle.try_clone().map_err(|e| e.to_string())?)
            .map_err(|e| format!("AI.EVIDENCE_ROOT_IDENTITY_UNAVAILABLE: {e}"))?;
        if expected != actual { return Err("AI.EVIDENCE_ROOT_CHANGED".into()); }
        Ok(handle)
    }

    pub(super) fn path_identity(path: &Path) -> std::io::Result<same_file::Handle> {
        let file = OpenOptions::new().read(true).access_mode(READ_ATTRIBUTES)
            .share_mode(SHARE_READ_WRITE | 0x4)
            .custom_flags(0x0200_0000 | OPEN_REPARSE).open(path)?;
        same_file::Handle::from_file(file)
    }

    fn check_directory(handle: &File) -> Result<(), String> {
        let metadata = handle.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & REPARSE != 0 {
            return Err("AI.EVIDENCE_REPARSE_OR_NOT_DIRECTORY".into());
        }
        Ok(())
    }

    fn open_child(parent: &File, name: &str, disposition: u32, directory: bool, add_directory: bool, add_file: bool) -> Result<File, String> {
        if name.is_empty() || name == "." || name == ".." || name.contains(['\\', '/', ':', '\0']) {
            return Err("AI.EVIDENCE_INVALID_COMPONENT".into());
        }
        let mut chars: Vec<u16> = name.encode_utf16().collect();
        let len = u16::try_from(chars.len().checked_mul(2).ok_or("AI.EVIDENCE_NAME_TOO_LONG")?)
            .map_err(|_| "AI.EVIDENCE_NAME_TOO_LONG")?;
        let mut unicode = UnicodeString { length: len, maximum_length: len, buffer: chars.as_mut_ptr() };
        let mut attrs = ObjectAttributes { length: std::mem::size_of::<ObjectAttributes>() as u32,
            root: parent.as_raw_handle() as Handle, name: &mut unicode, attributes: 0x40 | 0x1000,
            security_descriptor: std::ptr::null_mut(), security_qos: std::ptr::null_mut() };
        let mut io = IoStatus { status_or_pointer: 0, information: 0 };
        let mut raw: Handle = std::ptr::null_mut();
        let access = SYNCHRONIZE | READ_ATTRIBUTES | if directory {
            0x1 | 0x20 | (if add_directory {0x4} else {0}) | (if add_file {0x2} else {0})
        } else { 0x2 | 0x0001_0000 }; // FILE_WRITE_DATA | DELETE for handle-relative commit.
        let options = SYNC_NONALERT | OPEN_REPARSE | if directory { DIRECTORY_FILE } else { NON_DIRECTORY_FILE };
        let status = unsafe { NtCreateFile(&mut raw, access, &mut attrs, &mut io, std::ptr::null_mut(), 0,
            SHARE_READ_WRITE, disposition, options, std::ptr::null_mut(), 0) };
        if status < 0 {
            let code = unsafe { RtlNtStatusToDosError(status) };
            return Err(format!("AI.EVIDENCE_OPEN_FAILED: {}", std::io::Error::from_raw_os_error(code as i32)));
        }
        if raw.is_null() { return Err("AI.EVIDENCE_OPEN_FAILED: null handle".into()); }
        let file = unsafe { File::from_raw_handle(raw as _) };
        if directory { check_directory(&file)?; }
        Ok(file)
    }

    pub(super) fn open_directory(parent: &File, name: &str, create: bool, add_directory: bool, add_file: bool) -> Result<File, String> {
        open_child(parent, name, if create {3} else {1}, true, add_directory, add_file) // FILE_OPEN_IF / FILE_OPEN
    }
    pub(super) fn create_file(parent: &File, name: &str) -> Result<File, String> {
        open_child(parent, name, 2, false, false, false) // FILE_CREATE: existing names never replaced.
    }

    pub(super) fn rename_file(file: &File, parent: &File, name: &str) -> Result<(), String> {
        // FILE_RENAME_INFORMATION, class 10. ReplaceIfExists remains false;
        // the destination is one name relative to the held evidence directory.
        #[repr(C)] struct RenameInfo { replace: u8, root: Handle, byte_len: u32, name: [u16; 128] }
        if name.is_empty() || name.contains(['\\', '/', ':', '\0']) { return Err("AI.EVIDENCE_INVALID_COMPONENT".into()); }
        let chars: Vec<u16> = name.encode_utf16().collect();
        if chars.len() > 128 { return Err("AI.EVIDENCE_NAME_TOO_LONG".into()); }
        let mut info = RenameInfo { replace: 0, root: parent.as_raw_handle() as Handle,
            byte_len: (chars.len() * 2) as u32, name: [0; 128] };
        info.name[..chars.len()].copy_from_slice(&chars);
        let offset = (&info.name as *const _ as usize) - (&info as *const _ as usize);
        let mut io = IoStatus { status_or_pointer: 0, information: 0 };
        let status = unsafe { NtSetInformationFile(file.as_raw_handle() as Handle, &mut io,
            &mut info as *mut _ as *mut c_void, (offset + chars.len() * 2) as u32, 10) };
        if status < 0 {
            let code = unsafe { RtlNtStatusToDosError(status) };
            return Err(format!("AI.EVIDENCE_COMMIT_FAILED: {}; partial file may remain", std::io::Error::from_raw_os_error(code as i32)));
        }
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn exclusive_creation_keeps_existing_evidence_and_holds_directory_identity() {
        let root = tempfile::tempdir().unwrap();
        let destination = Destination::prepare(root.path(), "project-1").unwrap();
        let first = destination.write_new(b"first", "isf", None).unwrap();
        let second = destination.write_new(b"second", "isf", None).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
        let held = destination.directories.last().unwrap();
        let collision = first.file_name().unwrap().to_str().unwrap();
        assert!(windows::create_file(held, collision).is_err());
        let mut partial = windows::create_file(held, ".collision.partial").unwrap();
        partial.write_all(b"replacement").unwrap();
        partial.sync_all().unwrap();
        assert!(windows::rename_file(&partial, held, collision).is_err());
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert!(std::fs::rename(&destination.display_dir, root.path().join("moved-evidence")).is_err());
    }

    #[test]
    fn project_reparse_point_cannot_redirect_automatic_evidence() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let projects = root.path().join("projects");
        std::fs::create_dir(&projects).unwrap();
        let link = projects.join("project-1");
        // Directory junctions are reparse points and do not require the
        // symbolic-link privilege that is absent on many Windows systems.
        let status = std::process::Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"]).arg(&link).arg(outside.path())
            .status().expect("could not start junction creation");
        assert!(status.success(), "junction creation failed");
        assert!(Destination::prepare(root.path(), "project-1").is_err());
        assert!(!outside.path().join("ai-evidence").exists());
    }

    #[test]
    fn cancelled_before_write_does_not_create_an_artifact() {
        let root = tempfile::tempdir().unwrap();
        let destination = Destination::prepare(root.path(), "project-1").unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(true);
        assert!(destination.write_new(b"data", "isf", Some(&cancel)).is_err());
        assert_eq!(std::fs::read_dir(&destination.display_dir).unwrap().count(), 0);
    }

    #[test]
    fn preserves_native_bytes_and_rejects_path_like_extensions() {
        let root = tempfile::tempdir().unwrap();
        let destination = Destination::prepare(root.path(), "project-1").unwrap();
        let data = [0, 255, 128, 42];
        let saved = destination.write_new(&data, "wfm", None).unwrap();
        assert_eq!(saved.extension().unwrap(), "wfm");
        assert_eq!(std::fs::read(saved).unwrap(), data);
        assert!(destination.write_new(&data, "../wfm", None).is_err());
    }
}
