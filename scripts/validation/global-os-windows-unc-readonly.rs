//! Metadata-only comparison of the exact shared native open and Win32 open.
//! This program never creates a share, file, directory or file contents.
use std::{ffi::OsString, fs::File, io::{self, Write}, os::windows::{ffi::OsStringExt, fs::OpenOptionsExt, io::AsRawHandle}, path::{Path, PathBuf}};
#[repr(C)] #[derive(Default)] struct FileTime { low: u32, high: u32 }
#[repr(C)] #[derive(Default)] struct Information { attributes: u32, creation: FileTime, access: FileTime, write: FileTime, volume: u32, size_high: u32, size_low: u32, links: u32, index_high: u32, index_low: u32 }
#[link(name="kernel32")]
unsafe extern "system" {
    fn GetFinalPathNameByHandleW(handle: *mut std::ffi::c_void, path: *mut u16, size: u32, flags: u32) -> u32;
    fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Information) -> i32;
}
fn final_path(file: &File, flags: u32) -> io::Result<PathBuf> {
    let mut buffer=vec![0u16;256];loop{
        let length=unsafe{GetFinalPathNameByHandleW(file.as_raw_handle(),buffer.as_mut_ptr(),buffer.len() as u32,flags)} as usize;
        if length==0{return Err(io::Error::last_os_error());}if length<buffer.len(){return Ok(OsString::from_wide(&buffer[..length]).into());}
        if length>32768{return Err(io::Error::new(io::ErrorKind::InvalidData,"oversized handle path"));}buffer.resize(length+1,0);
    }
}
fn information(file:&File)->io::Result<(String,u32)>{let mut info=Information::default();if unsafe{GetFileInformationByHandle(file.as_raw_handle(),&mut info)}==0{return Err(io::Error::last_os_error());}Ok((format!("{:08x}:{:08x}{:08x}",info.volume,info.index_high,info.index_low),info.attributes))}
fn inspect(path:&Path)->io::Result<String>{
    let native=nomi_native_files::windows::open(None,path)?;
    let standard=std::fs::OpenOptions::new().access_mode(0x80).custom_flags(0x02000000|0x00200000).open(path)?;
    let native_path=final_path(&native,0)?;let standard_path=final_path(&standard,0)?;
    let native_metadata=native.metadata()?;let standard_metadata=standard.metadata()?;
    let (native_id,native_attributes)=information(&native)?;let (standard_id,standard_attributes)=information(&standard)?;
    Ok(format!("{{\"opened_path\":{:?},\"std_canonical\":{:?},\"native_final\":{:?},\"standard_final\":{:?},\"native_opened_name\":{:?},\"standard_opened_name\":{:?},\"native_path_equals_expected\":{},\"native_and_standard_final_equal\":{},\"native_id\":{:?},\"standard_id\":{:?},\"native_and_standard_id_equal\":{},\"native_attributes\":{},\"standard_attributes\":{},\"native_is_directory\":{},\"native_is_symlink\":{},\"standard_is_directory\":{},\"standard_is_symlink\":{}}}",path.to_string_lossy(),std::fs::canonicalize(path)?.to_string_lossy(),native_path.to_string_lossy(),standard_path.to_string_lossy(),final_path(&native,8)?.to_string_lossy(),final_path(&standard,8)?.to_string_lossy(),native_path==path,native_path==standard_path,native_id,standard_id,native_id==standard_id,native_attributes,standard_attributes,native_metadata.is_dir(),native_metadata.file_type().is_symlink(),standard_metadata.is_dir(),standard_metadata.file_type().is_symlink()))
}
fn main()->io::Result<()>{
    let root=std::env::args_os().nth(1).map(PathBuf::from).ok_or_else(||io::Error::new(io::ErrorKind::InvalidInput,"exact extended UNC root required"))?;
    let value=root.to_string_lossy();if !value.starts_with(r"\\?\UNC\localhost\NomiFun_GlobalOS_")||root.file_name().is_some(){return Err(io::Error::new(io::ErrorKind::InvalidInput,"only the generated share root is accepted"));}
    let target=nomi_native_files::windows::join_component(&root,std::ffi::OsStr::new("sample.txt"))?;
    let expected=target.parent().ok_or_else(||io::Error::new(io::ErrorKind::InvalidInput,"target has no parent"))?;
    let canonical_root=std::fs::canonicalize(&root)?;
    let body=format!("{{\"schema\":\"nomifun.windows-unc-readonly-native.v1\",\"target\":{:?},\"expected_parent\":{:?},\"std_canonical_root\":{:?},\"expected_parent_equals_canonical_root\":{},\"expected_parent_components\":{:?},\"canonical_root_components\":{:?},\"without_trailing_separator\":{},\"expected_parent_handle\":{},\"filesystem_mutations\":0}}\n",target.to_string_lossy(),expected.to_string_lossy(),canonical_root.to_string_lossy(),expected==canonical_root,format!("{:?}",expected.components().collect::<Vec<_>>()),format!("{:?}",canonical_root.components().collect::<Vec<_>>()),inspect(&canonical_root)?,inspect(expected)?);
    let mut output=io::stdout().lock();output.write_all(body.as_bytes())?;output.flush()
}
