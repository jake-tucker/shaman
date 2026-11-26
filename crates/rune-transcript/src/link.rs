//! The tokio side: a `ManagedClient` on its own threads, asked things over one channel and answering over whichever
//! bridge into Rune's main thread is being tried.

use std::{path::PathBuf, time::Instant};

use anyhow::{Context, Result};
use shaman::{
    IdentityStoreChoice,
    managed_client::{ManagedClient, ManagedClientConfig, SessionEvent, SessionSubscription},
    protocol::{SessionId, SessionListItem, SessionSnapshot, SessionUpdate},
};
use tokio::sync::mpsc;

/// The scratch Channel the prototype's Server serves. Never `release` or `debug`: a Client of another build replaces
/// the Server it finds, and those Channels hold the user's own.
pub const CHANNEL: &str = "rune-transcript";

#[derive(Clone)]
pub struct Listed {
    pub id: SessionId,
    pub title: String,
}

pub enum Ask {
    Open(SessionId, Instant),
    Stream {
        session_id: SessionId,
        words: u64,
        per_second: f64,
        chunk: u64,
    },
}

pub enum Said {
    Sessions(Vec<Listed>),
    Snapshot {
        snapshot: Box<SessionSnapshot>,
        asked: Instant,
        arrived: Instant,
    },
    /// Every update that was already waiting when this was handed over: one where each event crosses alone.
    Updates(Vec<SessionUpdate>),
    Problem(String),
}

/// How events cross to the UI thread, where the difference is the link's to make.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Crossing {
    /// One crossing per event.
    Each,
    /// One crossing for every event already waiting, as `SessionSubscription::try_next` is there for.
    Batch,
}

/// The directory beneath which the scratch Channel keeps its state and its copy of the Sessions.
pub fn state_base() -> Result<PathBuf> {
    std::env::var_os("RT_STATE")
        .map(PathBuf::from)
        .context("set RT_STATE to the scratch directory holding `rune-transcript/shaman.db`")
}

pub async fn connect() -> Result<ManagedClient> {
    let server = std::env::current_exe()?.with_file_name(format!(
        "rune-transcript-server{}",
        std::env::consts::EXE_SUFFIX
    ));
    anyhow::ensure!(
        server.exists(),
        "build `rune-transcript-server` beside this: {server:?}"
    );
    let config = ManagedClientConfig::new(state_base()?, CHANNEL)?
        .with_server_executable(server)
        .with_identity_store(IdentityStoreChoice::File);
    ManagedClient::connect(config).await
}

pub async fn list(client: &ManagedClient) -> Result<Vec<Listed>> {
    Ok(client
        .list_sessions(None)
        .await?
        .into_iter()
        .map(|item| match item {
            SessionListItem::Readable(summary) => Listed {
                id: summary.session.id,
                title: summary.title,
            },
            SessionListItem::Unreadable(summary) => Listed {
                id: summary.id,
                title: summary.title,
            },
        })
        .collect())
}

/// Asks the scratch Server's scripted Agent for a Turn, by the file it watches for.
pub fn ask_for_stream(
    session_id: SessionId,
    words: u64,
    per_second: f64,
    chunk: u64,
) -> Result<()> {
    let request = serde_json::json!({
        "session_id": session_id,
        "words": words,
        "per_second": per_second,
        "chunk": chunk,
    });
    let dir = state_base()?.join(CHANNEL);
    let staged = dir.join("stream-request.json.tmp");
    std::fs::write(&staged, request.to_string())?;
    std::fs::rename(staged, dir.join("stream-request.json"))?;
    Ok(())
}

/// Runs the link on a thread of its own until `asks` closes. `say` is called from tokio's threads.
pub fn start(
    crossing: Crossing,
    mut asks: mpsc::UnboundedReceiver<Ask>,
    say: impl Fn(Said) + Send + Sync + 'static,
) {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a tokio runtime");
        runtime.block_on(async move {
            if let Err(error) = run(crossing, &mut asks, &say).await {
                say(Said::Problem(format!("{error:#}")));
            }
        });
    });
}

async fn run(
    crossing: Crossing,
    asks: &mut mpsc::UnboundedReceiver<Ask>,
    say: &(impl Fn(Said) + Send + Sync),
) -> Result<()> {
    let mut client = connect().await?;
    say(Said::Sessions(list(&client).await?));
    let mut subscription: Option<(SessionSubscription, Instant)> = None;
    let mut managed_open = true;
    loop {
        tokio::select! {
            ask = asks.recv() => match ask {
                None => return Ok(()),
                Some(Ask::Open(session_id, asked)) => {
                    subscription = Some((client.subscribe_session(session_id).await?, asked));
                }
                Some(Ask::Stream { session_id, words, per_second, chunk }) => {
                    ask_for_stream(session_id, words, per_second, chunk)?;
                }
            },
            // What the Server says of itself is taken and dropped: an unread channel would hold its stream up.
            event = client.next(), if managed_open => managed_open = event.is_some(),
            event = next(&mut subscription) => {
                let Some((events, asked)) = subscription.as_mut() else { continue };
                let asked = *asked;
                match event {
                    None => subscription = None,
                    Some(Err(error)) => say(Said::Problem(format!("{error}"))),
                    Some(Ok(SessionEvent::Snapshot(snapshot))) => {
                        say(Said::Snapshot { snapshot, asked, arrived: Instant::now() });
                    }
                    Some(Ok(SessionEvent::Updated(update))) => {
                        let mut updates = vec![update];
                        let mut after = None;
                        if crossing == Crossing::Batch {
                            while let Some(Some(next)) = events.try_next() {
                                match next {
                                    Ok(SessionEvent::Updated(update)) => updates.push(update),
                                    other => { after = Some(other); break }
                                }
                            }
                        }
                        say(Said::Updates(updates));
                        match after {
                            Some(Ok(SessionEvent::Snapshot(snapshot))) => {
                                say(Said::Snapshot { snapshot, asked, arrived: Instant::now() });
                            }
                            Some(Err(error)) => say(Said::Problem(format!("{error}"))),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}

async fn next(
    subscription: &mut Option<(SessionSubscription, Instant)>,
) -> Option<Result<SessionEvent, shaman::managed_client::SessionStreamError>> {
    match subscription {
        Some((events, _)) => events.next().await,
        None => std::future::pending().await,
    }
}
