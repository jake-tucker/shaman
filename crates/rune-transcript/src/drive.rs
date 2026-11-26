//! `RT_AUTO`: drives the real window through the ticket's cases with nobody at it, and reports what each frame cost
//! as Rune's own frame hook measured it. What it cannot say is how any of it looks or feels.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use rune::AppHandle;
use shaman::protocol::SessionId;

use crate::{
    Bridge,
    link::Ask,
    view::{State, Timings},
};

pub fn start(handle: AppHandle<State>, timings: Arc<Mutex<Timings>>, bridge: Bridge) {
    std::thread::spawn(move || {
        let wanted: Option<SessionId> = std::env::var("RT_SESSION")
            .ok()
            .and_then(|id| serde_json::from_value(serde_json::Value::String(id)).ok());
        let report = |line: String| {
            eprintln!("{line}");
            if let Some(path) = std::env::var_os("RT_REPORT") {
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                {
                    let _ = writeln!(file, "{line}");
                }
            }
        };
        let phase = |name: &str| {
            let mut timings = timings.lock().expect("timings");
            let line = format!(
                "- {name}: {}; {} crossings carrying {} updates",
                frames(&timings.frames),
                timings.crossings,
                timings.updates
            );
            timings.frames.clear();
            timings.crossings = 0;
            timings.updates = 0;
            line
        };
        std::thread::sleep(Duration::from_secs(2));
        report(format!(
            "### window, bridge {bridge:?}, rss at rest {} MB",
            rss_mb()
        ));
        let _ = phase("start");

        handle.update(move |s: &mut State, _| {
            let id = wanted.or_else(|| s.sessions.first().map(|listed| listed.id));
            if let Some(id) = id {
                s.ask_open(id);
            }
        });
        let asked = Instant::now();
        while timings
            .lock()
            .expect("timings")
            .open_to_presented_ms
            .is_none()
            && asked.elapsed() < Duration::from_secs(20)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_secs(1));
        {
            let timings = timings.lock().expect("timings");
            report(format!(
                "- open: asked→first frame encoded {:.1} ms, →on the display {:.1} ms; rss {} MB",
                timings.open_to_encoded_ms.unwrap_or(f64::NAN),
                timings.open_to_presented_ms.unwrap_or(f64::NAN),
                rss_mb()
            ));
        }
        handle.update(|s: &mut State, _| eprintln!("- {}", s.note));
        report(phase("open, its frames"));

        // A scroll stepped as a fast wheel turns it, 240 pt a step. Each step is an update, so each frame here
        // also commits, which a wheel's own frames do not: the headless wheel is the measure without that.
        let steps = 1000;
        let step = |y: f32| {
            handle.update(move |s: &mut State, tx| tx.scroll_to_instant(s.viewport(), 0.0, y));
            std::thread::sleep(Duration::from_millis(8));
        };
        step(0.0);
        std::thread::sleep(Duration::from_millis(500));
        let _ = phase("jump to the head");
        for at in 1..=steps {
            step(at as f32 * 240.0);
        }
        std::thread::sleep(Duration::from_millis(500));
        report(phase("stepped head→foot, 240 pt every 8 ms"));
        for at in (0..steps).rev() {
            step(at as f32 * 240.0);
        }
        std::thread::sleep(Duration::from_millis(500));
        report(format!(
            "{}; rss {} MB",
            phase("stepped foot→head"),
            rss_mb()
        ));
        step(1.0e9);
        std::thread::sleep(Duration::from_millis(500));
        let _ = phase("jump to the foot");

        for _ in 0..60 {
            handle.update(|s: &mut State, _| s.draft.insert("x"));
            std::thread::sleep(Duration::from_millis(33));
        }
        std::thread::sleep(Duration::from_millis(300));
        report(phase("60 keystrokes into the composer"));

        let stream = |words: u64, per_second: f64| {
            handle.update(move |s: &mut State, _| {
                if let (Some(open), Some(asks)) = (&s.open, &s.asks) {
                    let _ = asks.send(Ask::Stream {
                        session_id: open.id,
                        words,
                        per_second,
                        chunk: 1,
                    });
                }
            });
            // Until the Turn has been quiet for a while: the scripted Agent is slower than it is asked to be.
            let (mut seen, mut quiet) = (0, Instant::now());
            let began = Instant::now();
            while (seen == 0 || quiet.elapsed() < Duration::from_millis(2500))
                && began.elapsed() < Duration::from_secs(60)
            {
                std::thread::sleep(Duration::from_millis(100));
                let now = timings.lock().expect("timings").updates;
                if now != seen {
                    (seen, quiet) = (now, Instant::now());
                }
            }
            began.elapsed().as_secs_f64() - 2.5
        };
        let took = stream(400, 80.0);
        report(format!(
            "{}; over {took:.1} s",
            phase("stream 400 words at 80/s, at the foot")
        ));
        let took = stream(1200, 600.0);
        report(format!(
            "{}; over {took:.1} s",
            phase("stream 1200 words as fast as the Server sends, at the foot")
        ));

        handle.update(|s: &mut State, tx| tx.scroll_page(s.viewport(), -4));
        std::thread::sleep(Duration::from_secs(2));
        let _ = phase("paging back");
        let took = stream(400, 80.0);
        report(format!(
            "{}; over {took:.1} s",
            phase("stream 400 words at 80/s, scrolled back four pages")
        ));

        report(format!("- rss at the end {} MB", rss_mb()));
        handle.quit();
    });
}

/// Count, and the median, 95th percentile and worst of the update and of the encode, in milliseconds.
pub fn frames(frames: &[(f64, f64)]) -> String {
    if frames.is_empty() {
        return "no frames drawn".to_owned();
    }
    let spread = |mut values: Vec<f64>| {
        values.sort_by(f64::total_cmp);
        let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
        format!("p50 {:.2} p95 {:.2} max {:.2}", at(0.5), at(0.95), at(1.0))
    };
    format!(
        "{} frames; update ms {}; encode ms {}",
        frames.len(),
        spread(frames.iter().map(|frame| frame.0).collect()),
        spread(frames.iter().map(|frame| frame.1).collect()),
    )
}

/// This process's resident memory, as `ps` reports it.
pub fn rss_mb() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|kib| kib.trim().parse::<u64>().ok())
        .map_or(0, |kib| kib / 1024)
}
