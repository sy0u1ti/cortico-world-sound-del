use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub trait Journal {
    fn load(&self) -> Result<BTreeSet<String>>;
    fn save(&mut self, devices: &BTreeSet<String>) -> Result<()>;
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Document {
    version: u32,
    restore_unmuted: BTreeSet<String>,
}

pub struct FileJournal {
    path: PathBuf,
    _lock: File,
}

impl FileJournal {
    pub fn open(directory: &Path) -> Result<Self> {
        if !directory.is_absolute() {
            bail!("The recovery directory must be absolute");
        }
        fs::create_dir_all(directory).context("Cannot create recovery directory")?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("helper.lock"))?;
        lock.try_lock()
            .context("Another helper owns this recovery directory")?;
        Ok(Self {
            path: directory.join("recovery.json"),
            _lock: lock,
        })
    }
}

impl Journal for FileJournal {
    fn load(&self) -> Result<BTreeSet<String>> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(BTreeSet::new());
            }
            Err(error) => return Err(error.into()),
        };
        let mut bytes = Vec::new();
        file.take(1_048_577).read_to_end(&mut bytes)?;
        if bytes.len() > 1_048_576 {
            bail!("Recovery journal exceeds size limit");
        }
        let document: Document = serde_json::from_slice(&bytes)
            .context("Invalid recovery journal; no audio was changed")?;
        if document.version != 1
            || document.restore_unmuted.len() > 128
            || document
                .restore_unmuted
                .iter()
                .any(|id| id.is_empty() || id.len() > 4096 || id.contains('\0'))
        {
            bail!("Unsupported recovery journal; no audio was changed");
        }
        Ok(document.restore_unmuted)
    }

    fn save(&mut self, devices: &BTreeSet<String>) -> Result<()> {
        atomic_json(
            &self.path,
            &Document {
                version: 1,
                restore_unmuted: devices.clone(),
            },
        )
    }
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        use windows::core::PCWSTR;
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
            .context("Cannot commit recovery journal")?;
        }
    }
    #[cfg(not(windows))]
    fs::rename(temporary, path)?;
    Ok(())
}
