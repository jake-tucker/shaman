//! Throwaway probe for issue #801: what a Windows toast needs from an app the install script put in place.
//!
//! A Rune window that posts Rune's own notifications (`rune-notify`) and writes down what comes back. It is driven
//! by two files beside the executable, since a launch Windows makes itself (a clicked toast, a Start menu shortcut)
//! carries none of the probe's flags:
//!
//! - `probe.cfg`, read once at start, `key=value` a line: `mode` (`none`, `adopt` or `register`), `id`, `name`,
//!   `icon`, `scheme`, `single` and `front`.
//! - `probe.cmd.txt`, polled while the window is open and deleted once read, a command a line: `post <id> <title>`,
//!   `post-button <id> <title>`, `remove <id>`, `focused`, `front` and `quit`.
//!
//! Everything is appended to `probe.log`.

#![forbid(unsafe_code)]

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rune::tok::{self, type_};
use rune::{App, AppSpec, Cx, El, Tx, col, text};
use rune_notify::{Identity, Notification};

#[derive(Default)]
struct State {
    last: String,
}

struct Config {
    mode: String,
    id: String,
    name: String,
    icon: Option<String>,
    scheme: Option<String>,
    single: bool,
    front: bool,
}

fn beside(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("the probe's own path");
    exe.parent().expect("a directory").join(name)
}

fn log(line: impl AsRef<str>) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let stamp = format!("{}.{:03}", now.as_secs(), now.subsec_millis());
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(beside("probe.log"))
    {
        let _ = writeln!(file, "{stamp} pid={} {}", std::process::id(), line.as_ref());
    }
}

fn config() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let text = std::fs::read_to_string(beside("probe.cfg")).unwrap_or_default();
        let get = |key: &str| {
            text.lines()
                .filter_map(|line| line.split_once('='))
                .find(|(k, _)| k.trim() == key)
                .map(|(_, v)| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        Config {
            mode: get("mode").unwrap_or_else(|| "none".into()),
            id: get("id").unwrap_or_else(|| "ai.shaman.probe801".into()),
            name: get("name").unwrap_or_else(|| "Shaman probe".into()),
            icon: get("icon"),
            scheme: get("scheme"),
            single: get("single").as_deref() == Some("true"),
            front: get("front").as_deref() == Some("true"),
        }
    })
}

fn view(s: &State, _: &mut Cx<State>) -> El {
    let t = tok::theme(true);
    col()
        .w_full()
        .h_full()
        .center()
        .gap(12.0)
        .fill(t.bg)
        .child(text("Toast probe").font(type_::TITLE).color(t.text))
        .child(text(s.last.clone()).color(t.text))
}

fn open_url(s: &mut State, url: &str, _: &mut Tx) {
    let click = rune_notify::click_from_url(url);
    log(format!("on_open_url {url:?} click={click:?}"));
    s.last = format!("link: {url}");
    if config().front {
        rune_notify::bring_to_front();
        log("bring_to_front after a link");
    }
}

fn run_command(line: &str, handle: &rune::AppHandle<State>) {
    let mut parts = line.trim().splitn(3, ' ');
    let (verb, id, rest) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    match verb {
        "post" | "post-button" => {
            let mut n = Notification::new(id, rest)
                .body("Fix the build")
                .info("session", "ses_1");
            if verb == "post-button" {
                n = n.action("approve", "Approve");
            }
            let posted = rune_notify::post(&n);
            log(format!(
                "{verb} {id:?} {rest:?} -> {posted} (available {}, focused {})",
                rune_notify::available(),
                rune_notify::app_focused()
            ));
        }
        "remove" => {
            rune_notify::remove(&[id]);
            log(format!("remove {id:?}"));
        }
        "focused" => log(format!("app_focused {}", rune_notify::app_focused())),
        "front" => handle.update(|_, _| {
            rune_notify::bring_to_front();
            log("bring_to_front on command");
        }),
        "quit" => {
            log("quit on command");
            handle.quit();
        }
        "" => {}
        other => log(format!("unknown command {other:?}")),
    }
}

fn main() {
    let cfg = config();
    log(format!(
        "start exe={:?} args={:?} mode={} id={} name={:?} icon={:?} scheme={:?} single={} front={}",
        std::env::current_exe().ok(),
        std::env::args().skip(1).collect::<Vec<_>>(),
        cfg.mode,
        cfg.id,
        cfg.name,
        cfg.icon,
        cfg.scheme,
        cfg.single,
        cfg.front
    ));
    if let Some(scheme) = &cfg.scheme {
        rune::url_schemes([scheme.as_str()]);
    }
    if cfg.single && !rune::single_instance(&cfg.id) {
        log("another process of the app runs: this launch was handed to it");
        return;
    }

    let mut identity = Identity::new(cfg.id.clone(), cfg.name.clone());
    if let Some(icon) = &cfg.icon {
        identity = identity.icon(icon);
    }
    if let Some(scheme) = &cfg.scheme {
        identity = identity.activation(scheme.clone());
    }
    let taken = match cfg.mode.as_str() {
        "register" => rune_notify::register(&identity),
        "adopt" => rune_notify::adopt(&identity),
        _ => Ok(()),
    };
    log(format!(
        "identity by {}: {taken:?}, available {}",
        cfg.mode,
        rune_notify::available()
    ));

    let app = App::new(AppSpec::new(
        "Toast probe",
        (360.0, 200.0),
        State::default,
        view,
    ))
    .on_open_url(open_url)
    .on_ready(|_| log("ready"));
    let handle = app.handle();

    let clicked = handle.clone();
    rune_notify::on_click(move |click| {
        log(format!("on_click {click:?}"));
        clicked.update(move |s, _| {
            s.last = format!("click: {}", click.id);
            if config().front {
                rune_notify::bring_to_front();
                log("bring_to_front after a click");
            }
        });
    });

    let commanded = handle.clone();
    std::thread::spawn(move || {
        let path = beside("probe.cmd.txt");
        loop {
            std::thread::sleep(Duration::from_millis(200));
            let Ok(commands) = std::fs::read_to_string(&path) else {
                continue;
            };
            let _ = std::fs::remove_file(&path);
            for line in commands.lines() {
                run_command(line, &commanded);
            }
        }
    });

    app.run();
    log("exit");
}
