//! This device: the name the phone goes by, kept in the thin Client's own
//! file, and what "This device" shows of it.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The thin Client's own file in its directory, beside the records it keeps
/// in the Server's shapes.
pub(crate) const RECORD_FILE: &str = "thin-client.json";

/// The name a phone is first offered as, until it is given one.
const FIRST_NAME: &str = "iPhone";

/// What "This device" shows.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ThisDevice {
    /// The name this phone gives itself pairing with a Server.
    pub name: String,
    /// What a change to the name reaches.
    pub name_note: String,
    /// The version of the protocol the core speaks to a Server.
    pub protocol_version: u32,
}

impl ThisDevice {
    pub(crate) fn of(record: &DeviceRecord) -> Self {
        Self {
            name: record.name.clone(),
            name_note: "Servers paired from now on know this phone by this name. Those already \
                        paired keep the name they were given."
                .to_owned(),
            protocol_version: shaman_protocol::PROTOCOL_VERSION,
        }
    }
}

/// What the thin Client's own file holds.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct DeviceRecord {
    /// The name, as the Serving side would keep it.
    #[serde(default = "first_name")]
    pub name: String,
}

impl Default for DeviceRecord {
    fn default() -> Self {
        Self { name: first_name() }
    }
}

fn first_name() -> String {
    FIRST_NAME.to_owned()
}

/// The directory a thin Client keeps everything in.
#[derive(Clone)]
pub(crate) struct Records {
    directory: PathBuf,
}

impl Records {
    /// The records in `directory`, made where it is absent.
    pub fn open(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("make the directory {}", directory.display()))?;
        Ok(Self { directory })
    }

    fn file(&self) -> PathBuf {
        self.directory.join(RECORD_FILE)
    }

    /// What the file holds, or what a new phone starts with where there is
    /// none.
    pub fn read(&self) -> Result<DeviceRecord> {
        shaman_pairing::read_records(&self.file())
    }

    /// Replaces the file with `record`, whole or not at all.
    pub fn write(&self, record: &DeviceRecord) -> Result<()> {
        shaman_pairing::write_private_json(&self.file(), record)
    }
}
