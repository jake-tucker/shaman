//! This device: the name the phone goes by and the key it proves itself by,
//! kept in the thin Client's own file, and what "This device" shows of them.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::identity::KeptKey;

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
    /// The fingerprint of this phone's identity key, whole, as a Serving
    /// Server shows this phone among its Peers.
    pub fingerprint: String,
    /// Said where the phone has a new key and so must pair again with every
    /// Server it had, until it is dismissed.
    pub pair_again: Option<String>,
    /// The version of the protocol the core states to every Server.
    pub protocol_version: u32,
}

impl ThisDevice {
    /// What "This device" shows of `record`, on a phone speaking `protocol`.
    pub(crate) fn of(record: &DeviceRecord, protocol: shaman_pairing::PairingProtocol) -> Self {
        Self {
            name: record.name.clone(),
            name_note: "Servers paired from now on know this phone by this name. Those already \
                        paired keep the name they were given."
                .to_owned(),
            fingerprint: record
                .key
                .as_ref()
                .map(|key| key.fingerprint.clone())
                .unwrap_or_default(),
            pair_again: record.pair_again.then(|| {
                "This phone has a new identity key, so no server it was paired with knows it \
                 any more. Pair with each of them again."
                    .to_owned()
            }),
            protocol_version: protocol.version,
        }
    }
}

/// What the thin Client's own file holds.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct DeviceRecord {
    /// The name, as the Serving side would keep it.
    #[serde(default = "first_name")]
    pub name: String,
    /// The key every other record is of, where one has been taken up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<KeptKey>,
    /// Whether the phone is yet to be told it must pair again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pair_again: bool,
    /// The Remotes, by name, in the order they were last turned toward, the
    /// latest first: the Outlook stands at the first still held.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turned: Vec<String>,
    /// The Workspace last turned toward on each Remote, by the Remote's
    /// name: where a Session begun there begins.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub workspaces: BTreeMap<String, RememberedWorkspace>,
}

/// A Workspace of a Remote, as the Outlook remembers it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct RememberedWorkspace {
    /// Its path on the Remote, as the Remote spells it.
    pub path: PathBuf,
    /// Its name, as the Remote names it.
    pub name: String,
}

impl DeviceRecord {
    /// The record of a phone called `name`, and nothing else.
    pub fn named(name: String) -> Self {
        Self {
            name,
            key: None,
            pair_again: false,
            turned: Vec::new(),
            workspaces: BTreeMap::new(),
        }
    }

    /// Turns the Outlook toward the Remote of the Session `listed` speaks
    /// of, first among those turned toward. Its Workspace becomes the one
    /// remembered for that Remote, or, only where none is remembered, the
    /// Session's own.
    pub fn turn_toward(&mut self, listed: &crate::listing::Listed) {
        let workspace = listed
            .workspace
            .as_ref()
            .filter(|_| !self.workspaces.contains_key(&listed.remote))
            .map(|(path, name)| RememberedWorkspace {
                path: path.clone(),
                name: name.clone(),
            });
        self.turn(&listed.remote, workspace);
    }

    /// Turns the Outlook toward the Remote `remote`, first among those
    /// turned toward, and toward its Workspace `workspace` where one is
    /// given, which is remembered for it; given none, the one remembered
    /// stays.
    pub fn turn(&mut self, remote: &str, workspace: Option<RememberedWorkspace>) {
        self.turned.retain(|held| held != remote);
        self.turned.insert(0, remote.to_owned());
        if let Some(workspace) = workspace {
            self.workspaces.insert(remote.to_owned(), workspace);
        }
    }
}

impl DeviceRecord {
    /// Remembers whatever was remembered of the Remote `from` of the Remote
    /// `to`, which is what it is called now.
    pub fn rename(&mut self, from: &str, to: &str) {
        for turned in &mut self.turned {
            if turned == from {
                to.clone_into(turned);
            }
        }
        if let Some(workspace) = self.workspaces.remove(from) {
            self.workspaces.insert(to.to_owned(), workspace);
        }
    }

    /// Forgets whatever was remembered of the Remote `remote`.
    pub fn forget(&mut self, remote: &str) {
        self.turned.retain(|turned| turned != remote);
        self.workspaces.remove(remote);
    }
}

impl Default for DeviceRecord {
    fn default() -> Self {
        Self::named(first_name())
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
    /// The records in `directory`, made where it is absent, with any
    /// replacement a write left as the app was ended midway removed.
    pub fn open(directory: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("make the directory {}", directory.display()))?;
        let records = Self { directory };
        records.remove_stale_replacements();
        Ok(records)
    }

    /// Removes every file a write made to replace a record by rename and
    /// left behind, as the app being ended before the rename leaves one.
    /// Nothing writes while the core opens, so none is under way.
    fn remove_stale_replacements(&self) {
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".replacing-")
                && name.ends_with(".tmp")
                && let Err(error) = std::fs::remove_file(entry.path())
            {
                tracing::warn!("a replacement left behind was not removed: {error}");
            }
        }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
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

#[cfg(test)]
mod tests {
    use super::*;

    fn remembered(name: &str) -> RememberedWorkspace {
        RememberedWorkspace {
            path: PathBuf::from(name),
            name: name.to_owned(),
        }
    }

    #[test]
    fn a_replacement_a_write_left_behind_is_removed_as_the_records_open() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join(".replacing-a1b2c3.tmp"),
            b"{\"name\":",
        )
        .unwrap();
        std::fs::write(directory.path().join(RECORD_FILE), b"{}").unwrap();

        Records::open(directory.path().to_owned()).unwrap();
        let mut held = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        held.sort();
        assert_eq!(held, [RECORD_FILE]);
    }

    #[test]
    fn a_workspace_turned_toward_replaces_the_one_remembered_and_none_keeps_it() {
        let mut record = DeviceRecord::default();
        record.turn("desk", Some(remembered("dotfiles")));
        record.turn("lab", None);
        record.turn("desk", Some(remembered("shaman")));
        assert_eq!(record.turned, ["desk", "lab"]);
        assert_eq!(record.workspaces.get("desk"), Some(&remembered("shaman")));

        record.turn("desk", None);
        assert_eq!(
            record.workspaces.get("desk"),
            Some(&remembered("shaman")),
            "turning toward the Remote alone keeps its Workspace"
        );
        assert_eq!(record.workspaces.get("lab"), None);
    }

    #[test]
    fn what_was_remembered_of_a_remote_goes_with_its_name_and_is_forgotten_with_it() {
        let mut record = DeviceRecord::default();
        record.turn("lab", Some(remembered("notes")));
        record.turn("desk", Some(remembered("shaman")));

        record.rename("desk", "study");
        assert_eq!(record.turned, ["study", "lab"], "in its place");
        assert_eq!(record.workspaces.get("study"), Some(&remembered("shaman")));
        assert_eq!(record.workspaces.get("desk"), None);

        record.forget("study");
        assert_eq!(record.turned, ["lab"]);
        assert_eq!(record.workspaces.get("study"), None);
        assert_eq!(record.workspaces.get("lab"), Some(&remembered("notes")));
    }
}
