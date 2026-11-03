//! Client-side projection of one provider-neutral Session stream.

use anyhow::{Result, bail};

use crate::{
    protocol::{SessionId, SessionSnapshot, SessionUpdate},
    session_projection::apply_update,
};

#[derive(Clone, Debug)]
pub(crate) struct SessionProjection {
    snapshot: SessionSnapshot,
}

impl SessionProjection {
    pub(crate) fn new(snapshot: SessionSnapshot) -> Self {
        Self { snapshot }
    }

    pub(crate) fn snapshot(&self) -> &SessionSnapshot {
        &self.snapshot
    }

    pub(crate) fn session_id(&self) -> SessionId {
        self.snapshot.session.id
    }

    /// Applies the next update from the Session's stream. A snapshot read
    /// out of band, such as the one a submission reconciles against, may land
    /// ahead of the stream; the updates it already covers are passed over
    /// until the stream catches up to it. Answers whether the update landed.
    pub(crate) fn apply(&mut self, update: SessionUpdate) -> Result<bool> {
        if update.session_id != self.session_id() {
            bail!("Session update targeted a different Session");
        }
        if update.revision <= self.snapshot.revision {
            return Ok(false);
        }
        apply_update(&mut self.snapshot, &update)?;
        Ok(true)
    }
}
