//! What the Landing's three pickers list, which is the glossary's and the same in every variant: the Workspace
//! Picker's rows, the Worktree Selector's, and the Directory Browser's tree. Where each stands, a dialog, a popover
//! or a column in place, is every variant's own.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{IconButton, IconButtonVariant, Kit, Spinner, TextInput};

use crate::landing::checkout_glyph;
use crate::parts::{self, glyph};
use crate::place::{self, PLACES};
use crate::sessions;
use crate::{App, Picker, bits};

/// One row of a picker: a lead, a name, what trails it, and whether it is the one the reader is on.
fn pick(key: String, lead: El, name: El, trail: Vec<El>, current: bool, k: &Kit) -> El {
    let mut line = row()
        .key(key)
        .w_full()
        .min_h(30.0)
        .px(8.0)
        .py(5.0)
        .gap(8.0)
        .center_y()
        .radius(k.radius.sm)
        .child(row().w(16.0).justify_center().child(lead))
        .child(row().grow(1.0).min_w(0.0).child(name));
    for t in trail {
        line = line.child(t);
    }
    if current {
        line = line.fill(k.c.layer_03);
    }
    k.interactive(line).role("button")
}

fn badge(said: &str, ink: rune::Color, k: &Kit) -> El {
    text(said.to_owned()).font(k.ty.small).color(ink)
}

/// What a picker says of a choice it could not make.
fn note(s: &App, k: &Kit) -> Option<El> {
    let why = s.picker_note.as_ref()?;
    Some(
        row()
            .key("picker-note")
            .w_full()
            .px(8.0)
            .py(6.0)
            .gap(8.0)
            .radius(k.radius.sm)
            .fill(k.c.warning.bg)
            .child(
                text(why.clone())
                    .font(k.ty.small)
                    .color(k.c.text_base)
                    .wrap()
                    .shrink(),
            ),
    )
}

/// How the Workspace Picker leads on to directories no Workspace holds yet.
#[derive(Clone, Copy, PartialEq)]
pub enum Browse {
    /// The Directory Browser, in the window.
    Tree,
    /// The system's folder dialog, which reads this machine's disk alone.
    System,
    /// Not from here: the directories stand beside the Workspaces already.
    None,
}

/// The Workspace Picker's search field.
pub fn search(s: &App, k: &Kit) -> El {
    TextInput::new(&s.query)
        .label("Search workspaces")
        .placeholder("Search workspaces")
        .width(None)
        .leading_icon(&parts::SEARCH)
        .on_edit(|s: &mut App| &mut s.query)
        .on_key(|s: &mut App, ev: &rune::KeyEvent, _: &mut rune::Tx| crate::field_key(s, ev))
        .build(k)
        .key("workspace-search")
        .autofocus()
}

/// The Workspace Picker's rows: the Browse row while no query is typed, then the Workspaces of the Outlook's
/// Server with the current one first.
pub fn workspaces(s: &App, browse: Browse, roots: bool, k: &Kit) -> El {
    let c = &k.c;
    let query = s.query.text.trim();
    let mut list = col().key("workspaces").w_full().gap(1.0);
    if query.is_empty() && browse != Browse::None {
        let system = browse == Browse::System;
        let label = if system {
            format!("Browse {}…", sessions::machine(s.outlook))
        } else {
            "Browse directories…".to_owned()
        };
        let mut trail = Vec::new();
        if system {
            trail.push(badge("the system's folder dialog", c.text_faint, k));
        }
        trail.push(badge("⌘O", c.text_faint, k));
        list = list.child(
            pick(
                "browse-row".into(),
                glyph(parts::FOLDER_TREE, 13.0, c.icon_muted),
                text(label).font(k.ty.ui).color(c.text_base),
                trail,
                false,
                k,
            )
            .on_press_tx(move |s: &mut App, tx: &mut rune::Tx| {
                let _ = tx;
                s.picker = Some(Picker::Directory);
                s.from_picker = true;
                s.root_tree("");
            }),
        );
    }
    let offered = place::offered(s.outlook, s.start.workspace, query);
    for at in &offered {
        let at = *at;
        let p = &PLACES[at];
        let current = at == s.start.workspace;
        let mut name =
            row()
                .gap(8.0)
                .center_y()
                .min_w(0.0)
                .child(
                    text(p.name)
                        .font(k.ty.ui_medium)
                        .color(if p.unavailable.is_some() {
                            c.text_faint
                        } else {
                            c.text_base
                        }),
                );
        if roots && at != place::SIDEKICK {
            name = name.child(
                row().shrink().min_w(0.0).child(
                    text(place::root(at, s.outlook))
                        .font(k.ty.small)
                        .color(c.text_faint)
                        .truncate_head(),
                ),
            );
        }
        let mut trail = Vec::new();
        if p.bare {
            trail.push(badge("bare", c.text_faint, k));
        }
        if p.unavailable.is_some() {
            trail.push(badge("unavailable", c.warning.icon, k));
        }
        if current {
            trail.push(badge("current", c.text_muted, k));
        }
        list = list.child(
            pick(
                format!("workspace-{at}"),
                parts::icon_of(p.icon, shaman_icons::FOLDER, 14.0, c.icon_base),
                name,
                trail,
                current,
                k,
            )
            .on_press(move |s: &mut App| s.choose_workspace(at)),
        );
    }
    if offered.is_empty() {
        list = list.child(
            row().px(8.0).py(8.0).child(
                text("No workspaces found")
                    .font(k.ty.ui)
                    .color(c.text_faint),
            ),
        );
    }
    list.children(note(s, k))
}

/// Beneath the Workspace Picker's rows: the Description of the Workspace the reader is on, in lines it keeps whether
/// or not that Workspace has one, and the row that offers editing it.
pub fn description(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let p = &PLACES[s.start.workspace];
    let said = if p.description.is_empty() {
        text("No description yet")
            .font(k.ty.small)
            .color(c.text_faint)
    } else {
        text(p.description)
            .font(k.ty.small)
            .color(c.text_muted)
            .wrap()
            .shrink()
    };
    row()
        .key("workspace-description")
        .w_full()
        .h(52.0)
        .px(12.0)
        .gap(10.0)
        .center_y()
        .fill(c.layer_01)
        .border_sides([1.0, 0.0, 0.0, 0.0], c.border_muted)
        .child(row().grow(1.0).min_w(0.0).child(said))
        .child(
            k.interactive(
                row().px(6.0).py(2.0).radius(k.radius.sm).child(
                    text("Edit description")
                        .font(k.ty.small)
                        .color(c.text_muted),
                ),
            )
            .hit(),
        )
}

/// The Worktree Selector: the current location named above the list and not offered, then creating a new Worktree,
/// then every Worktree of the Repository with its Checkout State and its location.
pub fn worktrees(s: &App, wide: bool, k: &Kit) -> El {
    let c = &k.c;
    let p = &PLACES[s.start.workspace];
    let mut list = col().key("worktrees").w_full().gap(1.0);
    if !p.repository {
        return list.child(
            row().px(8.0).py(8.0).child(
                text("A repository is required")
                    .font(k.ty.ui)
                    .color(c.text_faint),
            ),
        );
    }
    if s.start.resolving {
        return list.child(
            row()
                .px(8.0)
                .py(8.0)
                .gap(8.0)
                .center_y()
                .child(Spinner::new().size(12.0).build(k))
                .child(
                    text("Reading that repository…")
                        .font(k.ty.ui)
                        .color(c.text_faint),
                ),
        );
    }
    if wide {
        let now = match s.worktree() {
            Some(w) => match s.start.subdir {
                Some(sub) => format!("{} · {} · {sub}", w.checkout, w.location),
                None => format!("{} · {}", w.checkout, w.location),
            },
            None => "no working copy chosen".to_owned(),
        };
        list = list.child(
            row()
                .key("worktree-now")
                .w_full()
                .px(8.0)
                .pb(8.0)
                .gap(8.0)
                .child(text("Now in").font(k.ty.small).color(c.text_faint))
                .child(
                    row()
                        .grow(1.0)
                        .min_w(0.0)
                        .child(text(now).font(k.ty.small).color(c.text_muted).truncate()),
                ),
        );
    }
    list = list.child(
        pick(
            "worktree-new".into(),
            glyph(shaman_icons::WORKTREE, 13.0, c.icon_base),
            text("New worktree").font(k.ty.ui_medium).color(c.text_base),
            if wide {
                vec![badge("made when you send", c.text_faint, k)]
            } else {
                Vec::new()
            },
            s.start.intent,
            k,
        )
        .on_press(|s: &mut App| s.choose_new_worktree()),
    );
    for (at, w) in place::worktrees(s.start.workspace).iter().enumerate() {
        let current = s.start.worktree == Some(at) && !s.start.intent;
        let ink = if w.unavailable {
            c.text_faint
        } else {
            c.text_base
        };
        let location = || {
            text(w.location)
                .font(k.ty.small)
                .color(c.text_faint)
                .truncate_head()
        };
        let branch = text(w.checkout).font(k.ty.ui).color(ink).truncate();
        // Where there is room the location trails the Checkout State; in a column it stands beneath it.
        let mut trail = Vec::new();
        let name = if wide {
            trail.push(row().max_w(230.0).min_w(0.0).child(location()));
            branch
        } else {
            col().w_full().min_w(0.0).child(branch).child(location())
        };
        if w.unavailable {
            trail.push(badge("unavailable", c.warning.icon, k));
        }
        if s.start.worktree == Some(at) {
            trail.push(badge("current", c.text_muted, k));
        }
        list = list.child(
            pick(
                format!("worktree-{at}"),
                glyph(checkout_glyph(w.linked, w.detached), 13.0, c.icon_muted),
                name,
                trail,
                current,
                k,
            )
            .on_press(move |s: &mut App| s.choose_worktree(at)),
        );
    }
    list.children(if s.picker == Some(Picker::Worktree) {
        note(s, k)
    } else {
        None
    })
}

/// The Directory Browser's path field: the tree's root, which the reader re-roots at its parent, and the partial
/// name that narrows the root's children.
pub fn path_field(s: &App, wide: bool, k: &Kit) -> El {
    let c = &k.c;
    let home = place::home(s.outlook);
    let root = if s.tree_root.is_empty() {
        format!("{home}/")
    } else {
        format!("{home}/{}/", s.tree_root)
    };
    let up = IconButton::new(&parts::UP)
        .variant(IconButtonVariant::GhostMuted)
        .label("Up to the parent directory")
        .shortcut("←")
        .disabled(s.tree_root.is_empty())
        .on_press(|s: &mut App| {
            let parent = place::DIRS
                .iter()
                .find(|d| d.path == place::parent(s.tree_root))
                .map_or("", |d| d.path);
            let was = s.tree_root;
            s.root_tree(parent);
            // The former root stays open, as it stood.
            s.dirs_open.insert(was);
            s.dir_focus = Some(was);
        })
        .build(k)
        .key("tree-up");
    let hidden = k
        .interactive(
            row().px(6.0).py(3.0).radius(k.radius.sm).child(
                text(if s.hidden {
                    "Hide hidden"
                } else {
                    "Show hidden"
                })
                .font(k.ty.small)
                .color(c.text_muted),
            ),
        )
        .role("button")
        .on_press(|s: &mut App| s.hidden = !s.hidden);
    row()
        .key("path-field")
        .w_full()
        .h(34.0)
        .pl(2.0)
        .pr(6.0)
        .gap(4.0)
        .center_y()
        .radius(k.radius.md)
        .border(1.0, c.border_base)
        .fill(c.layer_01)
        .child(up)
        .child(text(root).font(k.ty.mono).color(c.text_muted).nowrap())
        .child(
            row().grow(1.0).min_w(40.0).child(
                TextInput::new(&s.partial)
                    .label("Path")
                    .placeholder("name")
                    .plain()
                    .width(None)
                    .text_style(k.ty.mono)
                    .on_edit(|s: &mut App| &mut s.partial)
                    .build(k)
                    .key("path-partial"),
            ),
        )
        .children(wide.then_some(hidden))
}

/// The Directory Browser's tree: the root first, its children indented beneath it, and only directories.
pub fn tree(s: &App, k: &Kit) -> El {
    let c = &k.c;
    let mut list = col().key("tree").w_full().gap(1.0);
    let row_of = |path: &'static str, depth: usize, name: String, placed: Option<usize>| {
        let open = s.dirs_open.contains(path) || path == s.tree_root;
        let branches = place::has_children(path, s.hidden);
        let twist = if !branches || path == s.tree_root {
            el().w(12.0)
        } else {
            rune::icon(
                if open {
                    k.glyph.chevron_down
                } else {
                    k.glyph.caret_right
                },
                12.0,
            )
            .color(c.icon_muted)
        };
        let lead = match placed {
            Some(w) => parts::icon_of(PLACES[w].icon, shaman_icons::FOLDER, 13.0, c.icon_base),
            None => glyph(shaman_icons::FOLDER, 13.0, c.icon_muted),
        };
        let mut line = row()
            .key(format!("dir-{path}"))
            .w_full()
            .h(28.0)
            .pl(8.0 + depth as f32 * 16.0)
            .pr(8.0)
            .gap(6.0)
            .center_y()
            .radius(k.radius.sm)
            .child(twist)
            .child(lead)
            .child(
                row()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(name).font(k.ty.ui).color(c.text_base).truncate()),
            );
        // What the Server reads of it and what the Client knows of it.
        if let Some(w) = placed {
            let p = &PLACES[w];
            if p.bare {
                line = line.child(badge("bare", c.text_faint, k));
            } else if p.repository {
                line = line.child(
                    row()
                        .gap(4.0)
                        .center_y()
                        .child(glyph(shaman_icons::GIT_BRANCH, 11.0, c.text_faint))
                        .child(badge("main", c.text_faint, k)),
                );
            }
            if w == s.start.workspace {
                line = line.child(badge("current", c.text_muted, k));
            }
        }
        if s.dir_focus == Some(path) || (s.dir_focus.is_none() && path == s.tree_root) {
            line = line.fill(c.layer_03);
        }
        // The pointer focuses and opens or closes a row with one press and chooses it with two.
        k.interactive(line)
            .role("button")
            .on_press(move |s: &mut App| {
                s.dir_focus = Some(path);
                if path != s.tree_root && !s.dirs_open.remove(path) {
                    s.dirs_open.insert(path);
                }
            })
            .on_double_press(move |s: &mut App| s.choose_dir(path))
    };
    if !s.tree_root.is_empty() {
        let placed = place::DIRS
            .iter()
            .find(|d| d.path == s.tree_root)
            .and_then(|d| d.place);
        list = list.child(row_of(
            s.tree_root,
            0,
            place::leaf(s.tree_root).to_owned(),
            placed,
        ));
    }
    let lift = usize::from(!s.tree_root.is_empty());
    let rows = place::rows(s.tree_root, &s.dirs_open, s.hidden, s.partial.text.trim());
    for (d, depth) in &rows {
        list = list.child(row_of(
            d.path,
            depth + lift,
            place::leaf(d.path).to_owned(),
            d.place,
        ));
    }
    if rows.is_empty() {
        list = list.child(
            row().px(8.0).py(6.0).pl(30.0).child(
                text("No directories here")
                    .font(k.ty.small)
                    .color(c.text_faint),
            ),
        );
    }
    list.children(if s.picker == Some(Picker::Directory) {
        note(s, k)
    } else {
        None
    })
}

/// The directory a choice in the browser would take: the focused row, or the root.
pub fn focused(s: &App) -> &'static str {
    s.dir_focus.unwrap_or(s.tree_root)
}

/// The button that chooses the focused directory.
pub fn choose(s: &App, k: &Kit) -> El {
    let at = focused(s);
    let name = if at.is_empty() {
        place::home(s.outlook).to_owned()
    } else {
        place::leaf(at).to_owned()
    };
    bits::small_button(
        &format!("Start in {name}"),
        rune_kit::ButtonVariant::Contrast,
        "⏎",
        k,
    )
    .disabled(at.is_empty())
    .on_press(move |s: &mut App| s.choose_dir(focused(s)))
    .build(k)
    .key("tree-choose")
}

/// A list that scrolls within the height its holder gives it.
pub fn scrolled(key: &str, list: El) -> El {
    col()
        .key(key.to_owned())
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(list)
}
