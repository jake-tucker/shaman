//! `RT_MEASURE`: the ticket's numbers with no window. Each Session is read from the scratch Server through
//! `ManagedClient` and driven through Rune's headless `Runtime`, which runs the view, layout, text shaping and paint
//! list on the CPU and draws nothing: what a frame costs before the GPU.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use rune::{Env, Runtime};
use shaman::{
    managed_client::{ManagedClient, SessionEvent, SessionSubscription},
    protocol::{SessionChange, SessionId, SessionSnapshot},
};

use crate::{
    drive::rss_mb,
    link,
    model::Open,
    view::{self, Config, State},
};

const SIZE: (f32, f32) = (1180.0, 780.0);
const FRAME: f64 = 1.0 / 60.0;

fn tokio() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?)
}

fn wanted(name: &str) -> Vec<SessionId> {
    std::env::var(name)
        .unwrap_or_default()
        .split(',')
        .filter_map(|id| {
            serde_json::from_value(serde_json::Value::String(id.trim().to_owned())).ok()
        })
        .collect()
}

async fn snapshot(
    client: &ManagedClient,
    id: SessionId,
) -> Result<(SessionSubscription, SessionSnapshot)> {
    let mut events = client.subscribe_session(id).await?;
    match events.next().await.context("the stream ended")?? {
        SessionEvent::Snapshot(snapshot) => Ok((events, *snapshot)),
        SessionEvent::Updated(_) => bail!("a Session's stream opens with its snapshot"),
    }
}

/// The Session `RT_PRELOAD` names, read before the window's first frame.
pub fn preloaded(config: Config) -> Option<Open> {
    let id = wanted("RT_PRELOAD").into_iter().next()?;
    let tokio = tokio().ok()?;
    let read = tokio.block_on(async {
        let client = link::connect().await?;
        snapshot(&client, id).await.map(|(_, snapshot)| snapshot)
    });
    match read {
        Ok(snapshot) => Some(Open::new(snapshot, config.repeat)),
        Err(error) => {
            eprintln!("preload failed: {error:#}");
            None
        }
    }
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1e3
}

fn spread(mut values: Vec<f64>) -> String {
    if values.is_empty() {
        return "none".to_owned();
    }
    values.sort_by(f64::total_cmp);
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    format!(
        "n {} sum {:.0} p50 {:.2} p95 {:.2} max {:.2}",
        values.len(),
        values.iter().sum::<f64>(),
        at(0.5),
        at(0.95),
        at(1.0)
    )
}

fn frame(rt: &mut Runtime<State>, dt: f64) -> f64 {
    let began = Instant::now();
    rt.frame(dt);
    ms(began)
}

/// The top of the row keyed `key`, where it is mounted.
fn top(rt: &Runtime<State>, key: &str) -> Option<f32> {
    rt.presented(key).map(|(rect, _)| rect[1])
}

/// Wheels the Transcript until the row keyed `end` has stood still for a few frames, and answers each frame's cost.
fn wheel_to(rt: &mut Runtime<State>, end: &str, dy: f32) -> Vec<f64> {
    let (x, y) = (SIZE.0 * 0.6, SIZE.1 * 0.5);
    let mut costs = Vec::new();
    let mut still = 0;
    let mut last = None;
    while still < 4 && costs.len() < 200_000 {
        rt.wheel(x, y, dy);
        costs.push(frame(rt, FRAME));
        let now = top(rt, end);
        still = if now.is_some() && now == last {
            still + 1
        } else {
            0
        };
        last = now;
    }
    costs
}

pub fn run(config: Config) -> Result<()> {
    let tokio = tokio()?;
    let began = Instant::now();
    let client = tokio.block_on(link::connect())?;
    let listed = tokio.block_on(link::list(&client))?;
    println!(
        "## headless, {config:?}\n\n- connected and listed {} Sessions in {:.0} ms; rss {} MB",
        listed.len(),
        ms(began),
        rss_mb()
    );
    let mut ids = wanted("RT_SESSIONS");
    if ids.is_empty() {
        ids = listed.iter().map(|listed| listed.id).collect();
    }
    for id in ids {
        if let Err(error) = session(&tokio, &client, config, id) {
            println!("- {id}: failed: {error:#}");
        }
    }
    Ok(())
}

fn session(
    tokio: &tokio::runtime::Runtime,
    client: &ManagedClient,
    config: Config,
    id: SessionId,
) -> Result<()> {
    let rss_before = rss_mb();
    let asked = Instant::now();
    let (mut events, snapshot) = tokio.block_on(snapshot(client, id))?;
    let wire = ms(asked);
    let bytes = serde_json::to_vec(&snapshot)?.len();
    let longest_message = snapshot
        .messages
        .iter()
        .map(|m| m.content.len())
        .max()
        .unwrap_or(0);

    let indexing = Instant::now();
    let open = Open::new(snapshot, config.repeat);
    let indexed = ms(indexing);
    let (rows, messages, activities) = (
        open.rows.len(),
        open.snapshot.messages.len(),
        open.snapshot.activities.len(),
    );
    let (first_key, last_key) = (open.rows[0].key.clone(), open.rows[rows - 1].key.clone());
    let mut state = State::new(config);
    state.show(open);
    let viewport = state.viewport();

    let mut rt = Runtime::new(state, view::view, SIZE, Env::new(true, 2.0));
    let first = frame(&mut rt, 0.0);
    let second = frame(&mut rt, FRAME);
    let mounted = rt.elements().len();
    println!(
        "\n### {id}\n\n- {rows} rows ({messages} Messages, {activities} Activities), {:.2} MB as JSON, longest Message \
         {longest_message} bytes\n- asked→snapshot decoded {wire:.1} ms; rows indexed {indexed:.2} ms; first frame \
         {first:.1} ms, second {second:.2} ms; {mounted} elements mounted; shown at the foot: {}",
        bytes as f64 / 1e6,
        top(&rt, &last_key).is_some(),
    );

    if std::env::var_os("RT_SELECT").is_some() {
        return dragged(&mut rt);
    }
    if let Some(pages) = std::env::var("RT_PAGE_BACK")
        .ok()
        .and_then(|pages| pages.parse::<i32>().ok())
    {
        return paged_back(tokio, &mut events, &mut rt, id, pages);
    }

    // Which way a wheel turns the list is learned by trying: the foot's last row moves down when scrolling back.
    let before = top(&rt, &last_key);
    rt.wheel(SIZE.0 * 0.6, SIZE.1 * 0.5, 40.0);
    rt.frame(FRAME);
    let back = if top(&rt, &last_key) > before {
        240.0
    } else {
        -240.0
    };
    let up = wheel_to(&mut rt, &first_key, back);
    let reached_head = top(&rt, &first_key).is_some();
    let down = wheel_to(&mut rt, &last_key, -back);
    let reached_foot = top(&rt, &last_key).is_some();
    println!(
        "- wheel foot→head (240 pt a frame): {}; reached {reached_head}\n- wheel head→foot: {}; reached \
         {reached_foot}; rss {} MB (was {rss_before})",
        spread(up),
        spread(down),
        rss_mb()
    );

    let mut keys = Vec::new();
    for _ in 0..60 {
        rt.update(|s, _| s.draft.insert("x"));
        keys.push(frame(&mut rt, 0.0));
    }
    println!(
        "- a keystroke's commit (state changed outside the Transcript): {}",
        spread(keys)
    );

    // Streamed at the foot, each update landing in a frame of its own: the worst case, one commit per token.
    let (each, _) = stream(tokio, &mut events, &mut rt, id, 300, false, None)?;
    println!(
        "- stream 300 words at the foot, a commit per update: {}; at rest {} frames after the last",
        spread(each),
        rest(&mut rt)
    );
    let (batched, _) = stream(tokio, &mut events, &mut rt, id, 1500, true, None)?;
    let resting = rest(&mut rt);
    let followed = top(
        &rt,
        &rt.state
            .open
            .as_ref()
            .expect("open")
            .rows
            .last()
            .expect("rows")
            .key
            .clone(),
    );
    println!(
        "- stream 1500 words at the foot, a commit per 16 ms of updates: {}; at rest {resting} frames after the \
         last; the foot followed: {}",
        spread(batched),
        followed.is_some()
    );
    // One long Message: every commit hands Markdown the whole of it again, so the cost is watched as it grows.
    let (long, _) = stream(tokio, &mut events, &mut rt, id, 6000, true, None)?;
    let tenth = (long.len() / 10).max(1);
    println!(
        "- stream one 6000-word Message, a commit per 16 ms: first tenth {}; last tenth {}; at rest {} frames after",
        spread(long[..tenth].to_vec()),
        spread(long[long.len() - tenth..].to_vec()),
        rest(&mut rt)
    );

    // Scrolled back to the middle of the Transcript: what the reader is looking at must not move while the foot grows.
    let anchor = rt.state.open.as_ref().expect("open").rows[rows / 2]
        .key
        .clone();
    rt.update({
        let anchor = anchor.clone();
        move |_, tx| tx.scroll_into_view(anchor)
    });
    for _ in 0..120 {
        rt.frame(FRAME);
    }
    let was = top(&rt, &anchor).context("the middle row is in view")?;
    let (back, tops) = stream(tokio, &mut events, &mut rt, id, 300, false, Some(&anchor))?;
    for _ in 0..30 {
        rt.frame(FRAME);
    }
    let moved: Vec<String> = tops
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0] != pair[1])
        .map(|(at, pair)| format!("frame {}: {:+.1}", at + 1, pair[1] - pair[0]))
        .take(8)
        .collect();
    println!(
        "- stream 300 words scrolled back to the middle: {}; the row in view ({anchor}, at {was:.0} pt) ended {:?} pt \
         away; moves: {moved:?}",
        spread(back),
        top(&rt, &anchor).map(|now| now - was),
    );

    let selecting = Instant::now();
    rt.update(|_, tx| tx.select_all_text("rows"));
    rt.frame(FRAME);
    let selected = rt.selected_text().unwrap_or_default();
    println!(
        "- select all rows: {:.1} ms, {} bytes selected",
        ms(selecting),
        selected.len(),
    );
    let _ = viewport;
    Ok(())
}

/// Asks the scratch Server for a Turn and lands every update it sends. Answers each frame's cost.
fn stream(
    tokio: &tokio::runtime::Runtime,
    events: &mut SessionSubscription,
    rt: &mut Runtime<State>,
    id: SessionId,
    words: u64,
    batched: bool,
    watch: Option<&str>,
) -> Result<(Vec<f64>, Vec<f32>)> {
    link::ask_for_stream(id, words, 2000.0, 1)?;
    let mut costs = Vec::new();
    let mut tops = Vec::new();
    let mut settled = false;
    while !settled {
        let event = tokio
            .block_on(async { tokio::time::timeout(Duration::from_secs(20), events.next()).await })
            .context("the scripted Turn went quiet")?
            .context("the stream ended")??;
        let mut updates = vec![event];
        if batched {
            // What a 60 Hz frame would find waiting.
            std::thread::sleep(Duration::from_millis(16));
            while let Some(Some(next)) = events.try_next() {
                updates.push(next?);
            }
        }
        for event in updates {
            let SessionEvent::Updated(update) = event else {
                continue;
            };
            settled |= update
                .changes
                .iter()
                .any(|change| matches!(change, SessionChange::TurnStatusChanged { .. }));
            rt.update(|s, _| {
                if let Some(open) = &mut s.open
                    && let Err(error) = open.apply(&update)
                {
                    eprintln!("update refused: {error:#}");
                }
            });
        }
        costs.push(frame(rt, FRAME));
        if let Some(key) = watch {
            tops.push(top(rt, key).unwrap_or(f32::NAN));
        }
    }
    Ok((costs, tops))
}

/// `RT_PAGE_BACK`: pages back from the foot, then streams, watching every row then in view.
fn paged_back(
    tokio: &tokio::runtime::Runtime,
    events: &mut SessionSubscription,
    rt: &mut Runtime<State>,
    id: SessionId,
    pages: i32,
) -> Result<()> {
    rt.update(move |s, tx| tx.scroll_page(s.viewport(), -pages));
    for _ in 0..120 {
        rt.frame(FRAME);
    }
    let in_view = |rt: &Runtime<State>| -> Vec<(String, f32)> {
        rt.state
            .open
            .as_ref()
            .expect("open")
            .rows
            .iter()
            .filter_map(|row| top(rt, &row.key).map(|y| (row.key.clone(), y)))
            .filter(|(_, y)| (-200.0..SIZE.1).contains(y))
            .collect()
    };
    let before = in_view(rt);
    let watch = before
        .last()
        .map(|(key, _)| key.clone())
        .context("a row in view")?;
    let (_, tops) = stream(tokio, events, rt, id, 300, false, Some(&watch))?;
    for _ in 0..60 {
        rt.frame(FRAME);
    }
    let after = in_view(rt);
    let moved: Vec<String> = tops
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| pair[0] != pair[1])
        .map(|(at, pair)| format!("frame {}: {:+.1}", at + 1, pair[1] - pair[0]))
        .take(12)
        .collect();
    println!(
        "- paged back {pages}: in view before {before:?}\n  after {after:?}\n  moves of the lowest: {moved:?}"
    );
    Ok(())
}

/// Steps frames until nothing is animating, and answers how many that took.
fn rest(rt: &mut Runtime<State>) -> usize {
    let mut frames = 0;
    while rt.frame(FRAME) && frames < 3600 {
        frames += 1;
    }
    frames
}

/// `RT_SELECT`: one drag from a row in view down through the rows beneath it, then the copy key.
fn dragged(rt: &mut Runtime<State>) -> Result<()> {
    let clipboard = rune::clipboard::memory();
    rt.use_clipboard(&clipboard);
    let in_view: Vec<(String, [f32; 4])> = rt
        .state
        .open
        .as_ref()
        .expect("open")
        .rows
        .iter()
        .filter_map(|row| {
            rt.presented(&row.key)
                .map(|(rect, _)| (row.key.clone(), rect))
        })
        .filter(|(_, rect)| rect[1] > 40.0 && rect[1] + rect[3] < SIZE.1 - 70.0)
        .collect();
    let (first, last) = (
        in_view.first().context("rows in view")?,
        in_view.last().context("rows in view")?,
    );
    // The pointer goes down and comes up on text itself: the first text drawn in one row, the last in the other.
    let texts = |key: &str| -> Vec<[f32; 4]> {
        rt.elements()
            .into_iter()
            .filter(|element| {
                element.path.contains(key)
                    && element
                        .text
                        .as_deref()
                        .is_some_and(|t| !t.trim().is_empty())
            })
            .map(|element| element.rect)
            .collect()
    };
    let begin = *texts(&first.0).first().context("text in the first row")?;
    let end = *texts(&last.0).last().context("text in the last row")?;
    let from = (begin[0] + 30.0, begin[1] + begin[3].min(20.0) / 2.0);
    let to = (
        end[0] + end[2].min(240.0) - 4.0,
        end[1] + end[3] - end[3].min(20.0) / 2.0,
    );
    rt.pointer_move(from.0, from.1);
    rt.pointer_down(from.0, from.1);
    rt.frame(FRAME);
    for step in 1..=12 {
        let f = step as f32 / 12.0;
        rt.pointer_move(from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f);
        rt.frame(FRAME);
    }
    rt.pointer_up(to.0, to.1);
    rt.frame(FRAME);
    let selected = rt.selected_text().unwrap_or_default();
    rt.key(rune::KeyEvent {
        key: rune::Key::Command('c'),
        mods: rune::Mods {
            cmd: true,
            ..Default::default()
        },
    });
    rt.frame(FRAME);
    let copied = clipboard
        .text()
        .map(|text| text.to_string())
        .unwrap_or_default();
    println!(
        "- a drag from row {} to row {} ({} rows in view): {} bytes selected over {} lines; the copy key put {} \
         bytes on the clipboard, the same text: {}",
        first.0,
        last.0,
        in_view.len(),
        selected.len(),
        selected.lines().count(),
        copied.len(),
        copied == selected,
    );
    Ok(())
}
