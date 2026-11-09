//! Cross-process locks for future device-session ownership.
//!
//! A lock protects a canonical resource name only. Ordinary Windows COM opens
//! through SerialSession, Dongle COM UART, service UART capture, exact VISA
//! resource names, and explicitly serial-numbered FT4222 sessions cooperate, but
//! WinUSB, index-based FT4222 paths and aliases remain uncoordinated; this must not
//! be used to claim complete device exclusivity yet.
use std::{fs::{File,OpenOptions,TryLockError},io,path::{Path,PathBuf}};
use sha2::{Digest,Sha256};

#[derive(Debug)]
pub enum ResourceLockError { InvalidResource, Busy, Io(io::Error) }
impl std::fmt::Display for ResourceLockError {
    fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
        match self {Self::InvalidResource=>write!(f,"invalid resource"),Self::Busy=>write!(f,"resource is owned by another session"),Self::Io(e)=>write!(f,"resource lock failed: {e}")}
    }
}
impl std::error::Error for ResourceLockError {}

/// Normalise stable COM and VISA names. Physical USB alias resolution remains
/// a separate capability and is intentionally not guessed here.
pub fn canonical_resource(input:&str)->Result<String,ResourceLockError>{
    let trimmed=input.trim();
    if trimmed.is_empty()||trimmed.len()>1024||trimmed.chars().any(char::is_control){return Err(ResourceLockError::InvalidResource);}
    let upper=trimmed.to_ascii_uppercase();
    let name=upper.strip_prefix("\\\\.\\").unwrap_or(&upper);
    let com_number=name.strip_prefix("ASRL").and_then(|s|s.strip_suffix("::INSTR"))
        .or_else(||name.strip_prefix("COM"));
    if let Some(number)=com_number {
        if !number.is_empty()&&number.bytes().all(|b|b.is_ascii_digit()) {
            let number=number.parse::<u32>().map_err(|_|ResourceLockError::InvalidResource)?;
            if number==0 {return Err(ResourceLockError::InvalidResource);}
            return Ok(format!("COM{number}"));
        }
    }
    Ok(name.to_owned())
}

/// Holds an OS file lock until dropped; the lock file is never deleted or used
/// as a stale-owner signal. The OS releases its lock when the process exits.
pub struct ResourceLease { _file:File, key:String }
impl ResourceLease {
    pub fn acquire(root:&Path,resource:&str)->Result<Self,ResourceLockError>{
        let key=canonical_resource(resource)?;
        std::fs::create_dir_all(root).map_err(ResourceLockError::Io)?;
        let hash=Sha256::digest(key.as_bytes());
        let filename:String=hash.iter().map(|byte|format!("{byte:02x}")).collect();
        let file=OpenOptions::new().read(true).write(true).create(true).truncate(false).open(root.join(format!("{filename}.lock"))).map_err(ResourceLockError::Io)?;
        match file.try_lock(){Ok(())=>Ok(Self{_file:file,key}),Err(TryLockError::WouldBlock)=>Err(ResourceLockError::Busy),Err(TryLockError::Error(e))=>Err(ResourceLockError::Io(e))}
    }
    pub fn key(&self)->&str{&self.key}
}

/// All cooperating local processes must use the same per-user directory.
pub fn default_lock_root()->Result<PathBuf,ResourceLockError>{
    dirs::data_local_dir().map(|path|path.join("WiParse").join("resource-locks")).ok_or(ResourceLockError::Io(io::Error::new(io::ErrorKind::NotFound,"local application data unavailable")))
}

/// Acquire an interprocess lease for a Windows COM resource. Non-COM
/// transports are not yet covered; callers must not treat None as exclusive.
pub fn acquire_com_resource(resource:&str)->Result<Option<ResourceLease>,ResourceLockError>{
    let key=canonical_resource(resource)?;
    if is_com(&key){Ok(Some(ResourceLease::acquire(&default_lock_root()?,&key)?))}else{Ok(None)}
}
pub fn acquire_com_resource_at(root:&Path,resource:&str)->Result<Option<ResourceLease>,ResourceLockError>{
    let key=canonical_resource(resource)?;
    if is_com(&key){
        Ok(Some(ResourceLease::acquire(root,&key)?))
    }else{Ok(None)}
}
/// Exact VISA names share the same lock namespace as COM aliases. Different
/// aliases for one physical instrument still require explicit resolution.
pub fn acquire_visa_resource(resource:&str)->Result<ResourceLease,ResourceLockError>{
    ResourceLease::acquire(&default_lock_root()?,resource)
}
fn is_com(key:&str)->bool{key.strip_prefix("COM").is_some_and(|suffix|!suffix.is_empty()&&suffix.bytes().all(|b|b.is_ascii_digit()))}

#[cfg(test)]
mod tests {
    use super::*;
    fn cleanup(root:PathBuf){let parent=std::env::temp_dir().canonicalize().unwrap();let target=root.canonicalize().unwrap();assert!(target.starts_with(parent)&&target.file_name().is_some());std::fs::remove_dir_all(target).unwrap();}
    #[test]fn normalises_com_aliases_without_guessing_usb_identity(){
        assert_eq!(canonical_resource(" com3 ").unwrap(),"COM3");
        assert_eq!(canonical_resource("ASRL3::INSTR").unwrap(),"COM3");
        assert_eq!(canonical_resource("ASRL003::INSTR").unwrap(),"COM3");
        assert_eq!(canonical_resource("COM003").unwrap(),"COM3");
        assert_eq!(canonical_resource("\\\\.\\COM3").unwrap(),"COM3");
        assert!(canonical_resource("COM0").is_err());
        assert_ne!(canonical_resource("USB0::A::INSTR").unwrap(),canonical_resource("USB0::B::INSTR").unwrap());
        assert!(canonical_resource("bad\nname").is_err());
    }
    #[test]fn lock_is_exclusive_and_released_on_drop(){
        let root=std::env::temp_dir().join(format!("wiparse-lock-test-{}",uuid::Uuid::new_v4()));
        let first=ResourceLease::acquire(&root,"COM3").unwrap();
        assert!(matches!(ResourceLease::acquire(&root,"ASRL3::INSTR"),Err(ResourceLockError::Busy)));
        drop(first);
        let second=ResourceLease::acquire(&root,"COM3").unwrap();drop(second);
        cleanup(root);
    }
    #[test]fn explicit_ft4222_serial_is_fenced_across_case_variants(){
        let root=std::env::temp_dir().join(format!("wiparse-ft4222-test-{}",uuid::Uuid::new_v4()));
        let path=crate::usb::parse_usb_resource("bridge://ft4222/serial=ft1234").unwrap();
        let query=crate::usb::parse_usb_resource("bridge://ft4222?serial=FT1234").unwrap();
        let first=ResourceLease::acquire(&root,&format!("FT4222::{}",path.serial)).unwrap();
        assert!(matches!(ResourceLease::acquire(&root,&format!("FT4222::{}",query.serial)),Err(ResourceLockError::Busy)));
        drop(first);drop(ResourceLease::acquire(&root,"FT4222::FT1234").unwrap());cleanup(root);
    }
    #[test]fn exact_visa_resource_is_exclusive_without_guessing_aliases(){
        let root=std::env::temp_dir().join(format!("wiparse-visa-test-{}",uuid::Uuid::new_v4()));
        let first=ResourceLease::acquire(&root,"USB0::0x0699::0x0401::SERIAL::INSTR").unwrap();
        assert!(matches!(ResourceLease::acquire(&root,"usb0::0X0699::0X0401::serial::instr"),Err(ResourceLockError::Busy)));
        let distinct=ResourceLease::acquire(&root,"TCPIP0::192.0.2.10::INSTR").unwrap();
        drop(distinct);drop(first);cleanup(root);
    }
    #[test]fn com_sessions_share_the_same_lease_and_other_transports_are_not_claimed(){
        let root=std::env::temp_dir().join(format!("wiparse-com-test-{}",uuid::Uuid::new_v4()));
        let first=acquire_com_resource_at(&root,"COM003").unwrap().unwrap();
        assert!(matches!(acquire_com_resource_at(&root,"ASRL3::INSTR"),Err(ResourceLockError::Busy)));
        assert!(acquire_com_resource_at(&root,"tcp://127.0.0.1:1234").unwrap().is_none());
        drop(first);drop(acquire_com_resource_at(&root,"COM3").unwrap());cleanup(root);
    }
    #[test]fn child_lock_probe(){
        let Some(root)=std::env::var_os("WIPARSE_RESOURCE_LOCK_TEST_DIR") else{return};
        assert!(matches!(ResourceLease::acquire(Path::new(&root),"ASRL7::INSTR"),Err(ResourceLockError::Busy)));
    }
    #[test]fn lock_blocks_a_separate_process(){
        let root=std::env::temp_dir().join(format!("wiparse-lock-process-test-{}",uuid::Uuid::new_v4()));
        let lease=ResourceLease::acquire(&root,"COM7").unwrap();
        let output=std::process::Command::new(std::env::current_exe().unwrap()).arg("--exact").arg("resource_lock::tests::child_lock_probe").arg("--nocapture").env("WIPARSE_RESOURCE_LOCK_TEST_DIR",&root).output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stdout));
        drop(lease);cleanup(root);
    }
    #[test]fn child_exit_probe(){
        let Some(root)=std::env::var_os("WIPARSE_RESOURCE_LOCK_EXIT_DIR") else{return};
        let _lease=ResourceLease::acquire(Path::new(&root),"COM8").unwrap();
        std::process::exit(23);
    }
    #[test]fn process_exit_releases_lock_without_removing_file(){
        let root=std::env::temp_dir().join(format!("wiparse-lock-exit-test-{}",uuid::Uuid::new_v4()));
        let status=std::process::Command::new(std::env::current_exe().unwrap()).arg("--exact").arg("resource_lock::tests::child_exit_probe").env("WIPARSE_RESOURCE_LOCK_EXIT_DIR",&root).status().unwrap();
        assert_eq!(status.code(),Some(23));
        let lease=ResourceLease::acquire(&root,"COM8").unwrap();drop(lease);cleanup(root);
    }
}
