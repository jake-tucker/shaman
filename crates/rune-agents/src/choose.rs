//! Choosing the Agent Selection and the Approval Posture from the readings under the composer, three ways.
//!
//! - `a`: the TUI's. The Provider and Model are one reading that opens the Models dialog, a list of every Provider's
//!   Models with a strip beneath it for the Model the pointer is on; a Model with Model Options leads on to a second
//!   step. The Model Options are a reading of their own that opens that step directly, and the Approval Posture is a
//!   third, with a dialog of its own.
//! - `b`: a menu from each reading. Every reading is a chip that opens a menu from itself: the Model, the reasoning
//!   effort, the Model's other options, the Approval Posture. One press opens, one chooses, nothing is confirmed.
//! - `c`: one picker over all of it. The whole line is one reading, and it opens one dialog: the Models down the
//!   side, and beside them what the Model the reader is on costs and holds, its Model Options, and the Approval
//!   Posture, all applied together.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, ControlSize, IconButton, IconButtonVariant, Kit, Menu, MenuItem,
    Placement, SegmentedControl, TextInput,
};

use crate::agents::{self, Agent, Model, PROVIDERS};
use crate::parts::{self, glyph};
use crate::{App, Chip, Choosing, Variant, over, sessions};

const DIALOG: f32 = 580.0;

fn provider_glyph(p: usize, size: f32, ink: rune::Color) -> El {
    match shaman_icons::provider_glyph(PROVIDERS[p].id) {
        Some(mark) => glyph(mark, size, ink),
        None => el().w(size),
    }
}

/// What the reading says once a change made while a Turn works has yet to take: the Turn keeps the Agent it began with.
fn next_turn(s: &App, k: &Kit) -> Option<El> {
    s.agent_next.then(|| {
        text("from the next turn")
            .font(k.ty.small)
            .color(k.c.info.icon)
    })
}

/// The Agent Selection and Approval Posture under the composer's field.
pub fn readings(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    match s.variant {
        Variant::A => plain(s, k),
        Variant::B => chips(s, k, layer),
        Variant::C => one(s, k),
    }
}

/// Whether the Approval Posture may be changed from where the reader stands: the TUI changes it for an open
/// Session alone, since the override is the Session's own.
fn posture_opens(s: &App) -> bool {
    s.open.is_some()
}

// ---------------------------------------------------------------------------------------------------------------
// a: the TUI's readings, each a way in to a dialog.

fn plain(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let way = |key: &'static str, inner: El, opens: Option<Choosing>| {
        let line = row()
            .key(key)
            .px(4.0)
            .py(2.0)
            .gap(5.0)
            .center_y()
            .radius(k.radius.sm)
            .child(inner);
        match opens {
            Some(what) => k
                .interactive(line)
                .role("button")
                .on_press(move |s: &mut App| s.choose(what)),
            None => line,
        }
    };
    let word = |t: String| text(t).font(k.ty.small).color(c.text_muted);
    let agent = &s.agent;
    let mut line = row().key("readings").gap(2.0).center_y().child(way(
        "reading-agent",
        row()
            .gap(5.0)
            .center_y()
            .child(provider_glyph(agent.provider, 12.0, c.icon_muted))
            .child(word(format!(
                "{} · {}",
                agent.provider().name,
                agent.model().name
            ))),
        Some(Choosing::Models),
    ));
    if !agent.model().options.is_empty() {
        let said = agent.said();
        line = line
            .child(text("·").font(k.ty.small).color(c.text_faint))
            .child(way(
                "reading-options",
                word(if said.is_empty() {
                    "options".into()
                } else {
                    said.join(" · ")
                }),
                Some(Choosing::Options),
            ));
    }
    line = line.child(el().w(8.0)).child(way(
        "reading-posture",
        row()
            .gap(5.0)
            .center_y()
            .child(glyph(shaman_icons::activity::APPROVAL, 12.0, c.icon_muted))
            .child(word(agents::posture_reading(agent.provider, &s.posture))),
        posture_opens(s).then_some(Choosing::Posture),
    ));
    line.children(next_turn(s, k).map(|n| row().pl(6.0).child(n)))
}

// ---------------------------------------------------------------------------------------------------------------
// b: a chip for each reading, and a menu from each chip.

fn chip(s: &App, which: Chip, inner: El, opens: bool, k: &Kit) -> El {
    let c = &k.c;
    let open = s.choosing == Some(Choosing::Menu(which));
    let mut line = row()
        .key(format!("chip-{}", which.key()))
        .h(24.0)
        .px(7.0)
        .gap(5.0)
        .center_y()
        .radius(k.radius.md)
        .child(inner);
    if !opens {
        return line;
    }
    line = line
        .border(1.0, c.border_muted)
        .child(rune::icon(k.glyph.chevron_down, 11.0).color(c.icon_faint));
    if open {
        line = line.fill(c.layer_03);
    }
    k.interactive(line)
        .role("button")
        .expanded(open)
        .on_press(move |s: &mut App| {
            if s.choosing == Some(Choosing::Menu(which)) {
                s.close_choosing();
            } else {
                s.choose(Choosing::Menu(which));
            }
        })
}

fn menu(s: &App, which: Chip, trigger: El, label: &str, width: f32) -> Menu {
    Menu::new(format!("menu-{}", which.key()))
        .label(label)
        .trigger(trigger)
        .open(s.choosing == Some(Choosing::Menu(which)))
        .placement(Placement::TopStart)
        .min_width(width)
        .on_close(|s: &mut App| s.close_choosing())
}

fn chips(s: &App, k: &Kit, layer: &mut Vec<El>) -> El {
    let c = &k.c;
    let agent = &s.agent;
    let word = |t: String| text(t).font(k.ty.small).color(c.text_muted);
    let mut line = row().key("readings").gap(6.0).center_y();

    // The Model, led by its Provider. A Session's Provider is bound, so there the chip names the Model alone.
    let named = if s.bound() {
        agent.model().name.to_owned()
    } else {
        format!("{} · {}", agent.provider().name, agent.model().name)
    };
    let trigger = chip(
        s,
        Chip::Model,
        row()
            .gap(5.0)
            .center_y()
            .child(provider_glyph(agent.provider, 12.0, c.icon_muted))
            .child(word(named)),
        true,
        k,
    );
    let mut models = menu(s, Chip::Model, trigger, "Model", 360.0);
    let mut first = true;
    for (p, provider) in PROVIDERS.iter().enumerate() {
        if s.bound() && p != agent.provider {
            continue;
        }
        if !first {
            models = models.separator();
        }
        first = false;
        let out = s.unavailable(p);
        models = models.group_label(match out {
            Some((why, _)) => format!("{} · {why}", provider.name),
            None => provider.name.to_owned(),
        });
        if let Some((_, fix)) = out {
            models = models.item(
                MenuItem::new(format!("Retry {}", provider.name))
                    .description(fix)
                    .on_select(|s: &mut App| s.codex_out = false),
            );
        }
        for (m, model) in provider.models.iter().enumerate() {
            let mut item = MenuItem::new(model.name)
                .description(match model.warning {
                    Some(warning) => format!("! {warning}"),
                    None => format!(
                        "{} · {} context · {}",
                        model.price.0, model.context, model.about
                    ),
                })
                .checked(p == agent.provider && m == agent.model)
                .disabled(out.is_some())
                .on_select(move |s: &mut App| s.pick_model(p, m));
            if model.warning.is_some() {
                item = item.badge("!");
            } else if model.default {
                item = item.badge("default");
            }
            models = models.item(item);
        }
    }
    line = line.child(models.build(k, layer));

    // The reasoning effort has a chip of its own; whatever else the Model offers shares one.
    let options = agent.model().options;
    if let Some(effort) = options.first() {
        let trigger = chip(
            s,
            Chip::Effort,
            word(effort.choices[agent.opts[0]].to_owned()),
            true,
            k,
        );
        let mut efforts =
            menu(s, Chip::Effort, trigger, effort.label, 200.0).group_label(effort.label);
        for (v, choice) in effort.choices.iter().enumerate() {
            let mut item = MenuItem::new(*choice)
                .checked(agent.opts[0] == v)
                .on_select(move |s: &mut App| s.set_opt(0, v));
            if v == effort.default {
                item = item.badge("default");
            }
            efforts = efforts.item(item);
        }
        line = line.child(efforts.build(k, layer));
    }
    if options.len() > 1 {
        let said: Vec<&str> = agent.said().into_iter().skip(1).collect();
        let trigger = chip(
            s,
            Chip::More,
            word(if said.is_empty() {
                "Options".into()
            } else {
                said.join(" · ")
            }),
            true,
            k,
        );
        let mut more = menu(s, Chip::More, trigger, "Model options", 200.0);
        for (i, opt) in options.iter().enumerate().skip(1) {
            if i > 1 {
                more = more.separator();
            }
            more = more.group_label(opt.label);
            for (v, choice) in opt.choices.iter().enumerate() {
                more = more.item(
                    MenuItem::new(*choice)
                        .checked(agent.opts[i] == v)
                        .on_select(move |s: &mut App| s.set_opt(i, v)),
                );
            }
        }
        line = line.child(more.build(k, layer));
    }

    let opens = posture_opens(s);
    let trigger = chip(
        s,
        Chip::Posture,
        row()
            .gap(5.0)
            .center_y()
            .child(glyph(shaman_icons::activity::APPROVAL, 12.0, c.icon_muted))
            .child(word(agents::posture_reading(agent.provider, &s.posture))),
        opens,
        k,
    );
    if opens {
        let mut postures = menu(s, Chip::Posture, trigger, "Approval posture", 380.0);
        for (d, dim) in agent.provider().posture.iter().enumerate() {
            if d > 0 {
                postures = postures.separator();
            }
            postures = postures.group_label(dim.label);
            for (v, (value, means)) in dim.values.iter().enumerate() {
                postures = postures.item(
                    MenuItem::new(*value)
                        .description(*means)
                        .checked(s.posture[d] == v)
                        .on_select(move |s: &mut App| s.set_posture(d, v)),
                );
            }
        }
        postures = postures.separator().item(
            MenuItem::new("Follow server settings")
                .description(format!(
                    "{}, as {}'s setting has it",
                    agents::posture_reading(agent.provider, &agents::setting(agent.provider)),
                    agent.provider().name
                ))
                .disabled(!s.pinned)
                .on_select(|s: &mut App| s.reset_posture()),
        );
        line = line.child(postures.build(k, layer));
    } else {
        line = line.child(trigger);
    }
    line.children(next_turn(s, k))
}

// ---------------------------------------------------------------------------------------------------------------
// c: one reading for the whole line.

fn one(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let agent = &s.agent;
    let word = |t: String| text(t).font(k.ty.small).color(c.text_muted);
    let open = s.choosing == Some(Choosing::All);
    let mut line = row()
        .key("reading-all")
        .h(24.0)
        .px(7.0)
        .gap(6.0)
        .center_y()
        .radius(k.radius.md)
        .border(1.0, c.border_muted)
        .child(provider_glyph(agent.provider, 12.0, c.icon_muted))
        .child(word(agent.reading()))
        .child(el().w(1.0).h(12.0).fill(c.border_base))
        .child(glyph(shaman_icons::activity::APPROVAL, 12.0, c.icon_muted))
        .child(word(agents::posture_reading(agent.provider, &s.posture)))
        .child(rune::icon(k.glyph.chevron_down, 11.0).color(c.icon_faint));
    if open {
        line = line.fill(c.layer_03);
    }
    row()
        .key("readings")
        .gap(8.0)
        .center_y()
        .child(
            k.interactive(line)
                .role("button")
                .expanded(open)
                .on_press(|s: &mut App| s.choose(Choosing::All)),
        )
        .children(next_turn(s, k))
}

// ---------------------------------------------------------------------------------------------------------------
// The dialogs of a and c.

/// What stands over the window while the reader chooses: nothing in b, whose menus open from their chips.
pub fn overlays(s: &App, k: &Kit, layer: &mut Vec<El>) {
    let Some(what) = s.choosing else {
        return;
    };
    let sheet = match what {
        Choosing::Models => models_dialog(s, k),
        Choosing::Options => options_dialog(s, k),
        Choosing::Posture => posture_dialog(s, k),
        Choosing::All => all_dialog(s, k),
        Choosing::Menu(_) => return,
    };
    layer.push(rune_kit::scrim("choosing-scrim", |s: &mut App| s.close_choosing()).fill(k.c.scrim));
    layer.push(over::centred(sheet));
}

fn head(s: &App, name: &str, under: Option<String>, k: &Kit) -> El {
    let c = &k.c;
    let mut line = row()
        .w_full()
        .h(44.0)
        .pl(16.0)
        .pr(8.0)
        .gap(8.0)
        .center_y()
        .child(
            text(name.to_owned())
                .font(k.ty.body_medium)
                .color(c.text_base),
        );
    if let Some(under) = under {
        line = line.child(text(under).font(k.ty.small).color(c.text_muted));
    }
    if s.outlook.is_some() {
        line = line.child(
            text(format!("on {}", sessions::machine(s.outlook)))
                .font(k.ty.small)
                .color(c.text_muted),
        );
    }
    line.child(el().grow(1.0)).child(
        IconButton::new(k.glyph.close)
            .variant(IconButtonVariant::GhostMuted)
            .label("Close")
            .shortcut("esc")
            .on_press(|s: &mut App| s.close_choosing())
            .build(k),
    )
}

fn foot(k: &Kit) -> El {
    row()
        .w_full()
        .px(16.0)
        .py(10.0)
        .gap(8.0)
        .center_y()
        .border_sides([1.0, 0.0, 0.0, 0.0], k.c.border_muted)
}

fn hints(said: &str, k: &Kit) -> El {
    text(said.to_owned()).font(k.ty.small).color(k.c.text_faint)
}

fn search(s: &App, k: &Kit) -> El {
    TextInput::new(&s.model_query)
        .label("Search models")
        .placeholder("Search models")
        .width(None)
        .leading_icon(&parts::SEARCH)
        .on_edit(|s: &mut App| &mut s.model_query)
        .on_key(|s: &mut App, ev: &rune::KeyEvent, _: &mut rune::Tx| crate::field_key(s, ev))
        .build(k)
        .key("model-search")
        .autofocus()
}

/// Every Provider's Models under its heading, as the TUI's picker lists them: a Session's bound Provider alone once
/// a Turn has bound it, an unavailable Provider kept in its place with its reason and what to do about it.
fn model_list(s: &App, compact: bool, k: &Kit) -> El {
    let c = &k.c;
    let query = s.model_query.text.trim();
    let mut list = col().key("models").w_full().gap(1.0);
    let mut any = false;
    for (p, provider) in PROVIDERS.iter().enumerate() {
        if s.bound() && p != s.agent.provider {
            continue;
        }
        let shown = agents::listed(p, query);
        if shown.is_empty() {
            continue;
        }
        any = true;
        let out = s.unavailable(p);
        list = list.child(
            row()
                .w_full()
                .px(8.0)
                .pt(8.0)
                .pb(3.0)
                .gap(6.0)
                .center_y()
                .child(provider_glyph(p, 12.0, c.text_accent))
                .child(
                    text(provider.name)
                        .font(k.ty.small_medium)
                        .color(c.text_accent),
                ),
        );
        if let Some((why, fix)) = out {
            list = list.child(
                k.interactive(
                    col()
                        .key(format!("retry-{}", provider.id))
                        .w_full()
                        .px(8.0)
                        .py(5.0)
                        .gap(2.0)
                        .radius(k.radius.sm)
                        .child(
                            text(format!("Retry {}: {why}", provider.name))
                                .font(k.ty.ui)
                                .color(c.danger.icon),
                        )
                        .child(text(fix).font(k.ty.small).color(c.text_muted).wrap()),
                )
                .role("button")
                .on_press(|s: &mut App| s.codex_out = false),
            );
        }
        for m in shown {
            list = list.child(model_row(s, p, m, out.is_some(), compact, k));
        }
    }
    if !any {
        list = list.child(
            row()
                .px(8.0)
                .py(8.0)
                .child(text("No models found").font(k.ty.ui).color(c.text_faint)),
        );
    }
    if s.bound() {
        list = list.child(
            row().w_full().px(8.0).pt(10.0).child(
                row().grow(1.0).min_w(0.0).child(
                    text(format!(
                        "This session's first turn bound it to {}. Start a new session to use another Provider.",
                        s.agent.provider().name
                    ))
                    .font(k.ty.small)
                    .color(c.text_faint)
                    .wrap()
                    .shrink(),
                ),
            ),
        );
    }
    list
}

fn tag(said: &str, k: &Kit) -> El {
    text(said.to_owned()).font(k.ty.small).color(k.c.text_faint)
}

fn model_row(s: &App, p: usize, m: usize, out: bool, compact: bool, k: &Kit) -> El {
    let c = &k.c;
    let model = &PROVIDERS[p].models[m];
    let current = (p, m) == (s.agent.provider, s.agent.model);
    let on = s.focus_model == (p, m);
    let ink = if out { c.text_faint } else { c.text_base };
    let mut line = row()
        .key(format!("model-{}-{}", PROVIDERS[p].id, model.id))
        .w_full()
        .h(30.0)
        .pl(26.0)
        .pr(8.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.sm)
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .child(text(model.name).font(k.ty.ui_medium).color(ink).truncate())
                .children(current.then(|| tag("current", k)))
                .children(model.default.then(|| tag("default", k)))
                .children(out.then(|| tag("unavailable", k))),
        );
    if model.warning.is_some() {
        line = line.child(text("!").font(k.ty.ui_medium).color(c.warning.icon));
    }
    line = line.child(
        row()
            .w(34.0)
            .child(text(model.price.0).font(k.ty.small).color(c.text_muted)),
    );
    if !compact {
        line = line.child(
            row()
                .w(36.0)
                .justify_end()
                .child(text(model.context).font(k.ty.small).color(c.text_faint)),
        );
    }
    if on {
        line = line.fill(c.layer_03);
    }
    if out {
        return line;
    }
    k.interactive(line)
        .role("button")
        .on_hover(move |s: &mut App| {
            // In a's dialog the strip follows the pointer; c's pane follows a press, since it holds what is edited.
            if s.variant == Variant::A {
                s.focus_model = (p, m);
            }
        })
        .on_press(move |s: &mut App| s.pick_model(p, m))
}

/// The two lines the TUI's narrow picker keeps under its list for the Model the reader is on: what it is, then its
/// warning where it carries one and what it costs where it does not.
fn strip(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let (p, m) = s.focus_model;
    let model = &PROVIDERS[p].models[m];
    let second = match model.warning {
        Some(warning) => text(format!("! {warning}"))
            .font(k.ty.small)
            .color(c.warning.icon)
            .wrap(),
        None => text(format!(
            "In {} · Out {} · Cache {}/{} per million tokens · {} context",
            model.tokens[0], model.tokens[1], model.tokens[2], model.tokens[3], model.context
        ))
        .font(k.ty.small)
        .color(c.text_faint)
        .truncate(),
    };
    col()
        .key("model-strip")
        .w_full()
        .min_h(52.0)
        .px(16.0)
        .py(8.0)
        .gap(2.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
        .child(
            text(format!(
                "{} · {} · {} price",
                model.id, model.about, model.price.1
            ))
            .font(k.ty.small)
            .color(c.text_muted)
            .truncate(),
        )
        .child(second)
}

fn models_dialog(s: &App, k: &Kit) -> El {
    over::sheet("models-dialog", DIALOG, k)
        .child(head(s, "Models", None, k))
        .child(col().w_full().px(12.0).pb(8.0).child(search(s, k)))
        .child(
            col().w_full().h(316.0).px(8.0).pb(8.0).child(
                col()
                    .key("models-scroll")
                    .w_full()
                    .h_full()
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .child(model_list(s, false, k)),
            ),
        )
        .child(strip(s, k))
        .child(foot(k).child(hints("type to search · ⏎ select · esc close", k)))
}

/// One Model Option as a line: what it is, and its values to press.
fn option_line(agent: &Agent, i: usize, width: f32, k: &Kit) -> El {
    let opt = &agent.model().options[i];
    row()
        .key(format!("option-{i}"))
        .w_full()
        .gap(12.0)
        .center_y()
        .child(
            row()
                .w(130.0)
                .child(text(opt.label).font(k.ty.ui).color(k.c.text_base)),
        )
        .child(
            SegmentedControl::new(opt.choices, Some(agent.opts[i]))
                .label(opt.label)
                .width(Some(width))
                .on_pick(move |s: &mut App, at, _| s.set_opt(i, at))
                .build(k)
                .key(format!("option-{i}-values")),
        )
}

fn options_dialog(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let agent = &s.draft_agent;
    let mut lines = col().w_full().px(16.0).pb(14.0).gap(12.0);
    for i in 0..agent.model().options.len() {
        lines = lines.child(option_line(agent, i, 380.0, k));
    }
    if agent.model().options.is_empty() {
        lines = lines.child(
            text(format!("{} has no options to set.", agent.model().name))
                .font(k.ty.ui)
                .color(c.text_muted),
        );
    }
    over::sheet("options-dialog", DIALOG, k)
        .child(head(
            s,
            "Model options",
            Some(format!(
                "{} · {}",
                agent.model().name,
                agent.provider().name
            )),
            k,
        ))
        .child(lines)
        .child(
            foot(k)
                .child(hints("⌘⏎ apply · esc cancel", k))
                .child(el().grow(1.0))
                .child(
                    Button::new("Cancel")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(|s: &mut App| s.close_choosing())
                        .build(k)
                        .key("cancel"),
                )
                .child(
                    Button::new("Apply")
                        .variant(ButtonVariant::Contrast)
                        .control_size(ControlSize::Small)
                        .on_press(|s: &mut App| s.use_draft())
                        .build(k)
                        .key("apply"),
                ),
        )
}

/// One of a posture's native values as a row: its name as the Provider writes it, and what it means.
fn posture_row(
    d: usize,
    v: usize,
    on: bool,
    pinned: bool,
    k: &Kit,
) -> impl Fn(&'static str, &'static str) -> El + '_ {
    move |value, means| {
        let c = &k.c;
        let mut line = row()
            .key(format!("posture-{d}-{v}"))
            .w_full()
            .min_h(30.0)
            .px(8.0)
            .py(5.0)
            .gap(8.0)
            .center_y()
            .radius(k.radius.sm)
            .child(row().w(16.0).justify_center().child(if on {
                rune::icon(k.glyph.check_small, 12.0).color(c.icon_base)
            } else {
                el()
            }))
            .child(
                row()
                    .w(150.0)
                    .child(text(value).font(k.ty.mono).color(c.text_base)),
            )
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(means).font(k.ty.small).color(c.text_muted).truncate()),
            );
        // The fill marks what the Session itself chose; a value its Provider's Setting gives is only ticked.
        if on && pinned {
            line = line.fill(c.layer_03);
        }
        k.interactive(line)
            .role("button")
            .on_press(move |s: &mut App| s.set_posture(d, v))
    }
}

fn posture_dialog(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let provider = s.agent.provider();
    let mut list = col().w_full().px(8.0).pb(8.0).gap(1.0);
    for (d, dim) in provider.posture.iter().enumerate() {
        list = list.child(
            row()
                .w_full()
                .px(8.0)
                .pt(if d == 0 { 0.0 } else { 10.0 })
                .pb(3.0)
                .gap(8.0)
                .child(text(dim.label).font(k.ty.small_medium).color(c.text_accent))
                .child(text(dim.what).font(k.ty.small).color(c.text_faint)),
        );
        for (v, (value, means)) in dim.values.iter().enumerate() {
            list = list.child(posture_row(d, v, s.posture[d] == v, s.pinned, k)(
                value, means,
            ));
        }
    }
    let follows = agents::posture_reading(s.agent.provider, &agents::setting(s.agent.provider));
    let mut reset = row()
        .key("posture-reset")
        .w_full()
        .min_h(30.0)
        .px(8.0)
        .py(5.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.sm)
        .child(row().w(16.0).justify_center().child(if s.pinned {
            el()
        } else {
            rune::icon(k.glyph.check_small, 12.0).color(c.icon_base)
        }))
        .child(
            text("Follow server settings (reset)")
                .font(k.ty.ui)
                .color(c.text_base),
        )
        .child(el().grow(1.0))
        .child(text(follows).font(k.ty.small).color(c.text_faint));
    if !s.pinned {
        reset = reset.fill(c.layer_03);
    }
    list = list.child(el().w_full().h(1.0).fill(c.border_muted)).child(
        k.interactive(reset)
            .role("button")
            .on_press(|s: &mut App| s.reset_posture()),
    );
    over::sheet("posture-dialog", DIALOG, k)
        .child(head(
            s,
            "Approval posture",
            Some(format!("this session · {}", provider.name)),
            k,
        ))
        .child(list)
        .child(foot(k).child(hints("⏎ choose · esc close", k)))
}

// ---------------------------------------------------------------------------------------------------------------
// c: the one picker.

/// Everything a Model's catalog entry says, with what the reader may set for it.
fn detail(s: &App, model: &'static Model, k: &Kit) -> El {
    let c = &k.c;
    let agent = &s.draft_agent;
    let fact = |name: &str, value: String| {
        row()
            .w_full()
            .gap(8.0)
            .child(
                row()
                    .w(110.0)
                    .child(text(name.to_owned()).font(k.ty.small).color(c.text_faint)),
            )
            .child(text(value).font(k.ty.small).color(c.text_muted))
    };
    let label = |t: &str| {
        text(t.to_owned())
            .font(k.ty.small_medium)
            .color(c.text_muted)
    };
    let mut pane = col()
        .key("model-detail")
        .w_full()
        .padding(16.0)
        .gap(10.0)
        .child(
            col()
                .w_full()
                .gap(2.0)
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .child(provider_glyph(agent.provider, 14.0, c.icon_base))
                        .child(text(model.name).font(k.ty.body_medium).color(c.text_base))
                        .child(text(model.id).font(k.ty.mono).color(c.text_faint)),
                )
                .child(text(model.about).font(k.ty.ui).color(c.text_muted)),
        );
    if let Some(warning) = model.warning {
        pane = pane.child(
            row()
                .key("model-warning")
                .w_full()
                .px(10.0)
                .py(7.0)
                .gap(8.0)
                .radius(k.radius.md)
                .fill(c.warning.bg)
                .child(text("!").font(k.ty.ui_medium).color(c.warning.icon))
                .child(
                    row().grow(1.0).min_w(0.0).child(
                        text(warning)
                            .font(k.ty.ui)
                            .color(c.text_base)
                            .wrap()
                            .shrink(),
                    ),
                ),
        );
    }
    for note in model.notes {
        pane = pane.child(text(*note).font(k.ty.small).color(c.text_muted).wrap());
    }
    pane = pane.child(
        col()
            .w_full()
            .gap(3.0)
            .child(fact(
                "Price",
                format!("{} · {}", model.price.0, model.price.1),
            ))
            .child(fact(
                "Per million tokens",
                format!(
                    "in {} · out {} · cache read {} · cache write {}",
                    model.tokens[0], model.tokens[1], model.tokens[2], model.tokens[3]
                ),
            ))
            .child(fact(
                "Context window",
                format!("{} · output limit {}", model.context, model.output),
            ))
            .child(fact(
                "Images",
                if model.images {
                    "Accepted".into()
                } else {
                    "Not accepted".into()
                },
            )),
    );
    if !model.options.is_empty() {
        let mut options = col().w_full().gap(8.0).child(label("Model options"));
        for i in 0..model.options.len() {
            options = options.child(option_line(agent, i, 330.0, k));
        }
        pane = pane
            .child(el().w_full().h(1.0).fill(c.border_muted))
            .child(options);
    }
    // The posture is the Session's own override, so the picker sets it for an open Session alone.
    let provider = agent.provider();
    let mut posture = col().w_full().gap(8.0).child(
        row()
            .w_full()
            .gap(8.0)
            .center_y()
            .child(label("Approval posture"))
            .child(el().grow(1.0))
            .children((s.open.is_some() && s.draft_pinned).then(|| {
                Button::new("Follow server settings")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Mini)
                    .on_press(|s: &mut App| s.reset_posture())
                    .build(k)
                    .key("posture-reset")
            })),
    );
    if s.open.is_none() {
        posture = posture.child(
            text(format!(
                "{}, as {}'s setting has it. A session may override it once it exists.",
                agents::posture_reading(agent.provider, &agents::setting(agent.provider)),
                provider.name
            ))
            .font(k.ty.small)
            .color(c.text_muted)
            .wrap(),
        );
    } else {
        for (d, dim) in provider.posture.iter().enumerate() {
            let names: Vec<&str> = dim.values.iter().map(|v| v.0).collect();
            let at = s.draft_posture.get(d).copied().unwrap_or(dim.setting);
            posture = posture.child(
                col()
                    .w_full()
                    .gap(4.0)
                    .child(
                        SegmentedControl::new(&names, Some(at))
                            .label(dim.label)
                            .width(None)
                            .on_pick(move |s: &mut App, at, _| s.set_posture(d, at))
                            .build(k)
                            .key(format!("posture-{d}-values")),
                    )
                    .child(
                        text(format!("{}: {}", dim.label, dim.values[at].1))
                            .font(k.ty.small)
                            .color(c.text_faint),
                    ),
            );
        }
    }
    pane.child(el().w_full().h(1.0).fill(c.border_muted))
        .child(posture)
}

fn all_dialog(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let width = (s.win_w - 120.0).clamp(640.0, 900.0);
    let height = (s.win_h - 140.0).clamp(420.0, 560.0);
    let draft = &s.draft_agent;
    let side = col()
        .w(300.0)
        .h_full()
        .border_sides([0.0, 1.0, 0.0, 0.0], c.border_muted)
        .fill(c.layer_01)
        .child(col().w_full().padding(8.0).child(search(s, k)))
        .child(
            col()
                .key("models-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .px(6.0)
                .pb(8.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(model_list(s, true, k)),
        );
    let changed = *draft != s.agent || s.draft_posture != s.posture || s.draft_pinned != s.pinned;
    let body = col()
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .child(
            col()
                .key("detail-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(detail(s, draft.model(), k)),
        )
        .child(
            foot(k)
                .child(hints("type to search · ⌘⏎ use · esc close", k))
                .child(el().grow(1.0))
                .child(
                    Button::new("Cancel")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(|s: &mut App| s.close_choosing())
                        .build(k)
                        .key("cancel"),
                )
                .child(
                    Button::new(format!("Use {}", draft.model().name))
                        .variant(ButtonVariant::Contrast)
                        .control_size(ControlSize::Small)
                        .disabled(!changed)
                        .on_press(|s: &mut App| s.use_draft())
                        .build(k)
                        .key("apply"),
                ),
        );
    over::sheet("agent-dialog", width, k)
        .h(height)
        .child(head(s, "Agent", None, k))
        .child(
            row()
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
                .child(side)
                .child(body),
        )
}
