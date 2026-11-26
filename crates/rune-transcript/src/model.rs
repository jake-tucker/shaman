//! The Session as the view reads it: the Server's snapshot kept whole, and one row per Transcript item with a stable
//! key and a revision that moves only when that item's own content does.

use std::collections::HashMap;

use anyhow::Result;
use shaman::protocol::{
    Activity, ActivityId, MessageId, SessionChange, SessionId, SessionSnapshot, SessionUpdate,
    TranscriptItem, session_projection::land_update,
};

#[derive(Clone, Copy)]
pub enum Item {
    Message(usize),
    Activity(usize),
    Unknown,
}

pub struct Row {
    pub key: String,
    pub item: Item,
    /// Bumped whenever the item this row draws changed: what a `memo` around the row is versioned by.
    pub revision: u64,
}

pub struct Open {
    pub id: SessionId,
    pub snapshot: SessionSnapshot,
    pub rows: Vec<Row>,
    /// Where each Message and Activity sits in the snapshot, and which row draws it.
    message_at: HashMap<MessageId, usize>,
    activity_at: HashMap<ActivityId, usize>,
    message_row: Vec<usize>,
    activity_row: Vec<usize>,
    /// How many Transcript items already have rows.
    listed: usize,
    pub updates: u64,
}

impl Open {
    /// `repeat` tiles the Transcript that many times over, for a Session longer than any this machine holds. The
    /// tiles are the Client's own invention: only the last one follows the Server.
    pub fn new(snapshot: SessionSnapshot, repeat: usize) -> Self {
        let mut open = Self {
            id: snapshot.session.id,
            snapshot,
            rows: Vec::new(),
            message_at: HashMap::new(),
            activity_at: HashMap::new(),
            message_row: Vec::new(),
            activity_row: Vec::new(),
            listed: 0,
            updates: 0,
        };
        open.index();
        for tile in 0..repeat.max(1) - 1 {
            for at in 0..open.snapshot.transcript.len() {
                let (key, item) = open.row_for(at);
                open.rows.push(Row {
                    key: format!("t{tile}/{key}"),
                    item,
                    revision: 0,
                });
            }
        }
        open.list();
        open
    }

    /// Applies one update from the Server in place, and moves the revision of every row whose item it changed.
    pub fn apply(&mut self, update: &SessionUpdate) -> Result<()> {
        let landed = land_update(&mut self.snapshot, update)?;
        self.updates += 1;
        for (change, landed) in update.changes.iter().zip(landed) {
            let Some(at) = landed else { continue };
            let row = match change {
                SessionChange::MessageContentAppended { .. }
                | SessionChange::MessageModelChanged { .. }
                | SessionChange::MessageTruncated { .. }
                | SessionChange::MessageCompleted { .. }
                | SessionChange::MessageSuperseded { .. } => self.message_row.get(at),
                SessionChange::CommandOutputAppended { .. }
                | SessionChange::CommandOutputTruncated { .. }
                | SessionChange::CommandStatusChanged { .. }
                | SessionChange::FileChangeUpdated { .. }
                | SessionChange::FileChangeStatusChanged { .. }
                | SessionChange::ToolCallInputChanged { .. }
                | SessionChange::ToolCallOutputAppended { .. }
                | SessionChange::ToolCallOutputTruncated { .. }
                | SessionChange::ToolCallStatusChanged { .. } => self.activity_row.get(at),
                // Every other change that lands on an Activity (a Questionnaire settling, a Reasoning block
                // growing) is left undrawn here: the throwaway follows Messages, Commands and Tool Calls.
                _ => None,
            };
            if let Some(&row) = row
                && let Some(row) = self.rows.get_mut(row)
            {
                row.revision += 1;
            }
        }
        self.index();
        self.list();
        Ok(())
    }

    pub fn activity(&self, at: usize) -> &Activity {
        &self.snapshot.activities[at]
    }

    fn index(&mut self) {
        for at in self.message_at.len()..self.snapshot.messages.len() {
            self.message_at.insert(self.snapshot.messages[at].id, at);
        }
        for at in self.activity_at.len()..self.snapshot.activities.len() {
            self.activity_at
                .insert(self.snapshot.activities[at].id(), at);
        }
        self.message_row
            .resize(self.snapshot.messages.len(), usize::MAX);
        self.activity_row
            .resize(self.snapshot.activities.len(), usize::MAX);
    }

    fn row_for(&self, at: usize) -> (String, Item) {
        match &self.snapshot.transcript[at] {
            TranscriptItem::Message { message_id } => (
                format!("m{message_id}"),
                self.message_at
                    .get(message_id)
                    .map_or(Item::Unknown, |&at| Item::Message(at)),
            ),
            TranscriptItem::Activity { activity_id } => (
                format!("a{activity_id}"),
                self.activity_at
                    .get(activity_id)
                    .map_or(Item::Unknown, |&at| Item::Activity(at)),
            ),
            TranscriptItem::Unknown => (format!("u{at}"), Item::Unknown),
        }
    }

    fn list(&mut self) {
        for at in self.listed..self.snapshot.transcript.len() {
            let (key, item) = self.row_for(at);
            let row = self.rows.len();
            match item {
                Item::Message(at) => self.message_row[at] = row,
                Item::Activity(at) => self.activity_row[at] = row,
                Item::Unknown => {}
            }
            self.rows.push(Row {
                key,
                item,
                revision: 0,
            });
        }
        self.listed = self.snapshot.transcript.len();
    }
}
