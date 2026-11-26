//! Throwaway (#774): one Session read through `ManagedClient` from a running Server and drawn by Rune as plain rows.
//! Not a design. `README.md` beside this crate says what it is for and how to run it.

mod drive;
mod link;
mod measure;
mod model;
mod view;

use std::sync::{Arc, Mutex};

use rune::{App, AppSpec, FrameOutcome, Tx};
use tokio::sync::mpsc;

use link::{Crossing, Said};
use view::{Config, State, Timings};

/// How what the link says reaches Rune's state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bridge {
    /// `AppHandle::update` once per event.
    Update,
    /// `AppHandle::update` once for every event already waiting.
    Batch,
    /// A channel in the state, drained by `poll`, which the `Waker` from `setup` asks for.
    Poll,
}

impl Bridge {
    fn from_env() -> Self {
        match std::env::var("RT_BRIDGE").as_deref() {
            Ok("update") => Self::Update,
            Ok("poll") => Self::Poll,
            _ => Self::Batch,
        }
    }
}

fn main() {
    let config = Config::from_env();
    if std::env::var_os("RT_MEASURE").is_some() {
        if let Err(error) = measure::run(config) {
            eprintln!("measure failed: {error:#}");
            std::process::exit(1);
        }
        return;
    }
    let bridge = Bridge::from_env();
    let (asks, asked) = mpsc::unbounded_channel();
    let (inbox, waiting) = std::sync::mpsc::channel::<Said>();
    let timings = Arc::new(Mutex::new(Timings::default()));

    let mut state = State::new(config);
    state.asks = Some(asks.clone());
    state.timings = timings.clone();
    if bridge == Bridge::Poll {
        state.inbox = Some(waiting);
    }
    // A picture or a tree taken without a window shows a Session only if it is there before the first frame.
    if let Some(open) = measure::preloaded(config) {
        state.show(open);
    }
    let waker = state.waker.clone();

    let mut spec = AppSpec::new(
        "Rune transcript (throwaway)",
        (1180.0, 780.0),
        || unreachable!("the state is built in main"),
        view::view,
    );
    spec.setup = Some(|s: &mut State, waker| {
        let _ = s.waker.set(waker);
    });
    spec.poll = Some(|s: &mut State, _, tx: &mut Tx| s.drain(tx));

    let app = App::new(spec)
        .init(move || state)
        .on_frame_encoded({
            let timings = timings.clone();
            move |frame| {
                if frame.outcome() != FrameOutcome::Drawn {
                    return;
                }
                let mut timings = timings.lock().expect("timings");
                timings.frames.push((
                    frame.update_ms().unwrap_or(0.0),
                    frame.encode_ms().unwrap_or(0.0),
                ));
                if let Some(asked) = timings.opening.take() {
                    timings.open_to_encoded_ms = Some((frame.at() - asked).as_secs_f64() * 1e3);
                    timings.awaiting_present = Some((frame.seq(), asked));
                }
            }
        })
        .on_frame_presented({
            let timings = timings.clone();
            move |frame| {
                let mut timings = timings.lock().expect("timings");
                if let Some((seq, asked)) = timings.awaiting_present
                    && frame.seq() >= seq
                {
                    timings.awaiting_present = None;
                    timings.open_to_presented_ms = frame
                        .presented_at()
                        .map(|at| (at - asked).as_secs_f64() * 1e3);
                }
            }
        });
    let handle = app.handle();

    let crossing = if bridge == Bridge::Batch {
        Crossing::Batch
    } else {
        Crossing::Each
    };
    match bridge {
        Bridge::Poll => link::start(crossing, asked, move |said| {
            let _ = inbox.send(said);
            if let Some(wake) = waker.get() {
                wake();
            }
        }),
        Bridge::Update | Bridge::Batch => {
            let handle = handle.clone();
            link::start(crossing, asked, move |said| {
                handle.update(move |s: &mut State, tx| s.take(said, tx));
            });
        }
    }
    if std::env::var_os("RT_AUTO").is_some() {
        drive::start(handle, timings, bridge);
    }
    app.run();
}
