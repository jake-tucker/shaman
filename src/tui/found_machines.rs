//! The machines `/pair` found at the Relays its Server holds: what each Relay
//! answered, the one list that makes, and where Row Focus stands in it.
//!
//! Each Relay is asked on its own and answers when it does, so the list grows
//! as they land, under keys the reader may already be using. Row Focus is kept
//! on a machine rather than a place in the list, and the Invite field is the
//! other place it stands; it moves of itself only until the reader has touched
//! anything, and to the first machine found. An answer belongs to the asking
//! that sent it, so one left over from before the overlay closed or the reader
//! looked again lands nowhere.
//!
//! One key is one machine however many Relays list it. It sits in the group of
//! the first Relay, in the order the Server holds them, that it waits at, or
//! where it waits at none, the first that lists it, and its row names the
//! others. It reads **Serving** if it waits at any of them, since it can be
//! reached there, and is named as the Relay it sits under names it, so where
//! two Relays were told different hostnames, the row keeps to one and the
//! detail of the selected row says the others. A machine is **Remote already**
//! whatever else it is, and goes by the Remote's name. Nothing here is believed
//! (ADR-0064): choosing a row only says what to do next, and forms no Pairing.

use std::collections::{HashMap, HashSet};

use crate::{
    protocol::{RelayMachine, RelayMachines, RelayMachinesAnswer},
    serving::{folded, short_fingerprint},
};

use super::{connect_overlay::ConnectRequest, list_window::ListWindow};

/// What is shown of the machines found, and what Row Focus stands on.
#[derive(Clone, Debug, Default)]
pub(super) struct FoundMachines {
    /// The asking whose answers are taken, while one is.
    asked: Option<ConnectRequest>,
    /// The Relays asked, as the Server holds them, once they are known.
    relays: Vec<String>,
    /// What each answered, last. A Relay asked again goes on showing this
    /// until it answers.
    answers: HashMap<String, RelayMachinesAnswer>,
    /// The Relays yet to answer the asking in hand.
    awaited: HashSet<String>,
    /// All of that as it is shown.
    groups: Vec<MachineGroup>,
    /// The machine Row Focus stands on, by its fingerprint; on the Invite field
    /// where there is none.
    selected: Option<String>,
    /// Whether the reader has done anything here yet: from then on, no answer
    /// landing moves Row Focus.
    touched: bool,
    /// What choosing the selected machine said, until the next key.
    note: Option<String>,
    /// The window over the groups, line by line.
    window: ListWindow,
}

/// One Relay's part of what is shown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MachineGroup {
    pub(super) address: String,
    pub(super) standing: GroupStanding,
    pub(super) machines: Vec<FoundMachine>,
    /// Whether the Relay holds more than it listed.
    pub(super) cut_short: bool,
}

/// How a Relay's group stands beside whatever machines it shows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum GroupStanding {
    /// It has yet to answer for the first time.
    Asking,
    /// It listed what it listed: of those, `elsewhere` are shown in another
    /// Relay's group.
    Listed {
        elsewhere: usize,
    },
    NotListed,
    LoginNeeded,
    Unreachable(String),
}

/// A machine as a row shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FoundMachine {
    /// The identity key the Relays tie it to, by their word, as a join would
    /// name it.
    pub(super) key: String,
    pub(super) fingerprint: String,
    /// What it goes by here: the Remote's name, or the hostname its group's
    /// Relay was told.
    pub(super) name: String,
    /// The first characters of its fingerprint, where another row goes by
    /// that name however it is cased or composed.
    pub(super) told_apart: Option<String>,
    pub(super) standing: MachineStanding,
    /// The Relays it waits at now, which are the ways a short Invite could
    /// reach it by.
    pub(super) waiting_at: Vec<String>,
    /// The other Relays that list it, not waiting there.
    pub(super) also_at: Vec<String>,
    /// What else it was named, as another Relay was told.
    pub(super) also_named: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum MachineStanding {
    Serving,
    NotServing,
    /// A Pairing this Server holds, with the Remote so named, has its key.
    RemoteAlready(String),
}

impl MachineStanding {
    /// Where it comes among a Relay's rows: what can be chosen first.
    fn rank(&self) -> u8 {
        match self {
            Self::Serving => 0,
            Self::RemoteAlready(_) => 1,
            Self::NotServing => 2,
        }
    }
}

impl FoundMachines {
    /// Begins finding machines afresh, as the overlay opens on them: what was
    /// shown before goes, and only answers to `asked` are taken.
    pub(super) fn begin(&mut self, asked: ConnectRequest) {
        *self = Self {
            asked: Some(asked),
            ..Self::default()
        };
    }

    /// Stops taking answers and shows nothing, as the overlay leaves `/pair`.
    pub(super) fn end(&mut self) {
        *self = Self::default();
    }

    /// Asks every Relay again as `asked`, what each said last standing until
    /// it answers, and Row Focus where it is.
    pub(super) fn look_again(&mut self, asked: ConnectRequest) {
        self.asked = Some(asked);
        self.note = None;
    }

    /// The Relays to be asked as `asked` are `relays`, where that is the asking
    /// in hand.
    pub(super) fn relays_known(&mut self, asked: ConnectRequest, relays: Vec<String>) {
        if self.asked != Some(asked) {
            return;
        }
        self.answers.retain(|address, _| relays.contains(address));
        self.awaited = relays.iter().cloned().collect();
        self.relays = relays;
        self.settle();
    }

    /// Takes what a Relay answered `asked`, where that is the asking in hand:
    /// any other is from before this one and says nothing of it.
    pub(super) fn answered(&mut self, asked: ConnectRequest, machines: RelayMachines) {
        if self.asked != Some(asked) || !self.awaited.remove(&machines.address) {
            return;
        }
        self.answers.insert(machines.address, machines.answer);
        self.settle();
    }

    /// Works out what is shown from what has been answered, keeping Row Focus
    /// on its machine wherever that has come to stand.
    fn settle(&mut self) {
        self.groups = groups(&self.relays, &self.answers);
        let rows = || self.groups.iter().flat_map(|group| &group.machines);
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| !rows().any(|machine| machine.fingerprint == *selected))
        {
            self.selected = None;
        }
        if !self.touched && self.selected.is_none() {
            let first = rows()
                .find(|machine| machine.standing == MachineStanding::Serving)
                .or_else(|| rows().next())
                .map(|machine| machine.fingerprint.clone());
            if first.is_some() {
                self.selected = first;
                self.window.reveal();
            }
        }
    }

    pub(super) fn groups(&self) -> &[MachineGroup] {
        &self.groups
    }

    pub(super) fn window(&self) -> &ListWindow {
        &self.window
    }

    /// Whether any machine is shown.
    pub(super) fn any(&self) -> bool {
        self.groups.iter().any(|group| !group.machines.is_empty())
    }

    /// Whether any Relay is asked, and so whether looking again means
    /// anything.
    pub(super) fn any_relays(&self) -> bool {
        !self.relays.is_empty()
    }

    /// The machine Row Focus stands on, by fingerprint, where it stands on one.
    pub(super) fn selected_fingerprint(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The machine Row Focus stands on, where it stands on one.
    pub(super) fn selected(&self) -> Option<&FoundMachine> {
        let selected = self.selected.as_ref()?;
        self.groups
            .iter()
            .flat_map(|group| &group.machines)
            .find(|machine| machine.fingerprint == *selected)
    }

    pub(super) fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The reader did something on the Invite field: Row Focus stands there.
    pub(super) fn focus_invite(&mut self) {
        self.touched = true;
        self.selected = None;
        self.note = None;
    }

    /// Row Focus moves between the Invite field and the machines.
    pub(super) fn focus_next(&mut self) {
        if self.selected.is_some() {
            self.focus_invite();
        } else {
            self.walk(true);
        }
    }

    /// Row Focus moves one place back or on along the machines and the Invite
    /// field, which is the one place between the last machine and the first.
    pub(super) fn walk(&mut self, forward: bool) {
        self.touched = true;
        self.note = None;
        let rows: Vec<&str> = self
            .groups
            .iter()
            .flat_map(|group| &group.machines)
            .map(|machine| machine.fingerprint.as_str())
            .collect();
        let next = match (&self.selected, forward) {
            (None, true) => rows.first().copied(),
            (None, false) => rows.last().copied(),
            (Some(selected), _) => {
                let at = rows.iter().position(|row| row == selected);
                at.and_then(|at| {
                    if forward {
                        rows.get(at + 1).copied()
                    } else {
                        at.checked_sub(1).and_then(|at| rows.get(at).copied())
                    }
                })
            }
        }
        .map(str::to_owned);
        self.selected = next;
        self.window.reveal();
    }

    /// Chooses the machine Row Focus stands on, answering whether it stands on
    /// one. Nothing comes of it but being told what to do there: no machine
    /// listed is trusted, or asked anything, on the strength of being chosen.
    pub(super) fn choose(&mut self) -> bool {
        self.touched = true;
        let Some(machine) = self.selected() else {
            return false;
        };
        let name = machine.shown_name();
        self.note = Some(match &machine.standing {
            MachineStanding::Serving => {
                format!("To pair with {name}, issue an Invite there with /serve and paste it here")
            }
            MachineStanding::NotServing => format!(
                "{name} is logged in but not waiting to be reached, which may mean Serving is \
                 off, it is not Served through this Relay, or the machine is off; run /serve \
                 there, then issue an Invite to paste here"
            ),
            MachineStanding::RemoteAlready(remote) => {
                format!("Already paired as the Remote {remote}; /connect turns toward it")
            }
        });
        true
    }
}

impl FoundMachine {
    /// What it goes by, told apart from any other going by that.
    pub(super) fn shown_name(&self) -> String {
        match &self.told_apart {
            Some(told_apart) => format!("{} ({told_apart})", self.name),
            None => self.name.clone(),
        }
    }
}

/// What every Relay's answer comes to, one key one row.
fn groups(relays: &[String], answers: &HashMap<String, RelayMachinesAnswer>) -> Vec<MachineGroup> {
    /// One key as the Relays list it, in the order the Server holds them.
    struct Sighting<'a> {
        relay: &'a str,
        machine: &'a RelayMachine,
    }
    let mut order: Vec<&str> = Vec::new();
    let mut sightings: HashMap<&str, Vec<Sighting<'_>>> = HashMap::new();
    for relay in relays {
        let Some(RelayMachinesAnswer::Listed { machines, .. }) = answers.get(relay) else {
            continue;
        };
        for machine in machines {
            let seen = sightings.entry(machine.fingerprint.as_str()).or_default();
            if seen.is_empty() {
                order.push(machine.fingerprint.as_str());
            }
            seen.push(Sighting { relay, machine });
        }
    }
    let mut placed: HashMap<&str, Vec<FoundMachine>> = HashMap::new();
    for fingerprint in order {
        let seen = &sightings[fingerprint];
        let at = seen
            .iter()
            .find(|sighting| sighting.machine.waiting)
            .unwrap_or(&seen[0]);
        let remote = seen
            .iter()
            .find_map(|sighting| sighting.machine.remote.clone());
        let name = remote
            .clone()
            .unwrap_or_else(|| at.machine.hostname.clone());
        let mut also_named: Vec<String> = Vec::new();
        for sighting in seen {
            let hostname = &sighting.machine.hostname;
            if *hostname != name && !also_named.contains(hostname) {
                also_named.push(hostname.clone());
            }
        }
        let waiting_at: Vec<String> = seen
            .iter()
            .filter(|sighting| sighting.machine.waiting)
            .map(|sighting| sighting.relay.to_owned())
            .collect();
        placed.entry(at.relay).or_default().push(FoundMachine {
            key: at.machine.key.clone(),
            fingerprint: fingerprint.to_owned(),
            name,
            told_apart: None,
            standing: match remote {
                Some(remote) => MachineStanding::RemoteAlready(remote),
                None if waiting_at.is_empty() => MachineStanding::NotServing,
                None => MachineStanding::Serving,
            },
            also_at: seen
                .iter()
                .filter(|sighting| !sighting.machine.waiting)
                .map(|sighting| sighting.relay.to_owned())
                .filter(|relay| relay != at.relay)
                .collect(),
            waiting_at,
            also_named,
        });
    }
    let mut groups: Vec<MachineGroup> = relays
        .iter()
        .map(|relay| {
            let mut machines = placed.remove(relay.as_str()).unwrap_or_default();
            machines.sort_by_cached_key(|machine| {
                (
                    machine.standing.rank(),
                    folded(&machine.name),
                    machine.fingerprint.clone(),
                )
            });
            let (standing, cut_short) = match answers.get(relay) {
                None => (GroupStanding::Asking, false),
                Some(RelayMachinesAnswer::Listed {
                    machines: listed,
                    cut_short,
                }) => (
                    GroupStanding::Listed {
                        elsewhere: listed.len() - machines.len(),
                    },
                    *cut_short,
                ),
                Some(RelayMachinesAnswer::NotListed) => (GroupStanding::NotListed, false),
                Some(RelayMachinesAnswer::LoginNeeded) => (GroupStanding::LoginNeeded, false),
                Some(RelayMachinesAnswer::Unreachable { message }) => {
                    (GroupStanding::Unreachable(message.clone()), false)
                }
            };
            MachineGroup {
                address: relay.clone(),
                standing,
                machines,
                cut_short,
            }
        })
        .collect();
    tell_apart(&mut groups);
    groups
}

/// Gives each machine going by a name another also goes by, however it is
/// cased or composed, as many of the first characters of its fingerprint as
/// tell it from those others, as a Peer's name does.
fn tell_apart(groups: &mut [MachineGroup]) {
    let mut named: HashMap<String, Vec<String>> = HashMap::new();
    for machine in groups.iter().flat_map(|group| &group.machines) {
        named
            .entry(folded(&machine.name))
            .or_default()
            .push(machine.fingerprint.clone());
    }
    for machine in groups.iter_mut().flat_map(|group| &mut group.machines) {
        let fingerprints = &named[&folded(&machine.name)];
        if fingerprints.len() < 2 {
            continue;
        }
        let shown = (short_fingerprint(&machine.fingerprint).len()..=machine.fingerprint.len())
            .step_by(4)
            .map(|shown| &machine.fingerprint[..shown])
            .find(|shown| {
                fingerprints
                    .iter()
                    .filter(|other| other.starts_with(shown))
                    .count()
                    == 1
            })
            .unwrap_or(&machine.fingerprint);
        machine.told_apart = Some(shown.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = "https://one.example";
    const TWO: &str = "https://two.example";

    fn asking(number: u64) -> ConnectRequest {
        ConnectRequest::numbered(number)
    }

    fn machine(fingerprint: &str, hostname: &str, waiting: bool) -> RelayMachine {
        RelayMachine {
            key: format!("key-of-{fingerprint}"),
            fingerprint: fingerprint.to_owned(),
            hostname: hostname.to_owned(),
            waiting,
            remote: None,
        }
    }

    fn listed(address: &str, machines: Vec<RelayMachine>) -> RelayMachines {
        RelayMachines {
            address: address.to_owned(),
            answer: RelayMachinesAnswer::Listed {
                machines,
                cut_short: false,
            },
        }
    }

    fn found(relays: &[&str]) -> FoundMachines {
        let mut found = FoundMachines::default();
        found.begin(asking(1));
        found.relays_known(
            asking(1),
            relays.iter().map(|relay| (*relay).to_owned()).collect(),
        );
        found
    }

    fn rows(found: &FoundMachines) -> Vec<(String, MachineStanding)> {
        found
            .groups()
            .iter()
            .flat_map(|group| &group.machines)
            .map(|machine| (machine.shown_name(), machine.standing.clone()))
            .collect()
    }

    fn fingerprints(found: &FoundMachines, group: usize) -> Vec<&str> {
        found.groups()[group]
            .machines
            .iter()
            .map(|machine| machine.fingerprint.as_str())
            .collect()
    }

    #[test]
    fn each_machine_says_how_it_stands_what_can_be_chosen_coming_first() {
        let mut found = found(&[ONE]);
        let mut paired = machine("bbbb0000", "build-host", true);
        paired.remote = Some("build-box".to_owned());
        found.answered(
            asking(1),
            listed(
                ONE,
                vec![
                    machine("cccc0000", "old-laptop", false),
                    paired,
                    machine("aaaa0000", "workstation", true),
                ],
            ),
        );
        assert_eq!(
            rows(&found),
            [
                ("workstation".to_owned(), MachineStanding::Serving),
                (
                    "build-box".to_owned(),
                    MachineStanding::RemoteAlready("build-box".to_owned())
                ),
                ("old-laptop".to_owned(), MachineStanding::NotServing),
            ]
        );
        assert_eq!(
            found.selected().map(|machine| machine.name.as_str()),
            Some("workstation")
        );
    }

    #[test]
    fn a_key_two_relays_list_is_one_row_serving_where_it_waits_at_either() {
        let mut found = found(&[ONE, TWO]);
        found.answered(
            asking(1),
            listed(ONE, vec![machine("aaaa0000", "named-at-one", false)]),
        );
        assert_eq!(fingerprints(&found, 0), ["aaaa0000"]);
        assert_eq!(rows(&found)[0].1, MachineStanding::NotServing);

        found.answered(
            asking(1),
            listed(TWO, vec![machine("aaaa0000", "named-at-two", true)]),
        );
        assert_eq!(
            fingerprints(&found, 0),
            Vec::<&str>::new(),
            "it sits where it can be reached"
        );
        assert_eq!(fingerprints(&found, 1), ["aaaa0000"]);
        let row = &found.groups()[1].machines[0];
        assert_eq!(row.standing, MachineStanding::Serving);
        assert_eq!(row.name, "named-at-two");
        assert_eq!(row.waiting_at, [TWO]);
        assert_eq!(row.also_at, [ONE]);
        assert_eq!(row.also_named, ["named-at-one"]);
        assert_eq!(
            found.groups()[0].standing,
            GroupStanding::Listed { elsewhere: 1 }
        );
    }

    #[test]
    fn a_key_waiting_at_both_relays_sits_with_the_first_and_either_may_reach_it() {
        let mut found = found(&[ONE, TWO]);
        found.answered(
            asking(1),
            listed(TWO, vec![machine("aaaa0000", "box", true)]),
        );
        found.answered(
            asking(1),
            listed(ONE, vec![machine("aaaa0000", "box", true)]),
        );
        assert_eq!(fingerprints(&found, 0), ["aaaa0000"]);
        let row = &found.groups()[0].machines[0];
        assert_eq!(
            row.waiting_at,
            [ONE, TWO],
            "in the order the Server holds them"
        );
        assert_eq!(row.also_named, Vec::<String>::new());
    }

    #[test]
    fn a_remote_paired_by_a_key_either_relay_lists_is_a_remote_whatever_else_it_is() {
        let mut found = found(&[ONE, TWO]);
        let mut known = machine("aaaa0000", "hostname", false);
        known.remote = Some("the-remote".to_owned());
        found.answered(asking(1), listed(ONE, vec![known]));
        found.answered(
            asking(1),
            listed(TWO, vec![machine("aaaa0000", "hostname", true)]),
        );
        assert_eq!(
            rows(&found),
            [(
                "the-remote".to_owned(),
                MachineStanding::RemoteAlready("the-remote".to_owned())
            )]
        );
    }

    #[test]
    fn rows_going_by_one_name_show_the_first_characters_of_each_fingerprint() {
        let mut found = found(&[ONE, TWO]);
        found.answered(
            asking(1),
            listed(
                ONE,
                vec![
                    machine("aaaa00001111", "laptop", true),
                    machine("bbbb00001111", "LAPTOP", true),
                    machine("cccc00001111", "desktop", true),
                ],
            ),
        );
        found.answered(
            asking(1),
            listed(TWO, vec![machine("dddd00001111", "Lapto\u{70}", false)]),
        );
        let names: Vec<String> = rows(&found).into_iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            [
                "desktop",
                "laptop (aaaa0000)",
                "LAPTOP (bbbb0000)",
                "Laptop (dddd0000)"
            ],
            "the rows of every group tell apart, and a name no other goes by says nothing more"
        );
    }

    #[test]
    fn fingerprints_a_relay_chose_to_share_a_beginning_are_told_apart_further() {
        let mut found = found(&[ONE]);
        found.answered(
            asking(1),
            listed(
                ONE,
                vec![
                    machine("abcdef012345aaaa", "box", true),
                    machine("abcdef012345bbbb", "box", true),
                ],
            ),
        );
        let names: Vec<String> = rows(&found).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["box (abcdef012345aaaa)", "box (abcdef012345bbbb)"]);
    }

    /// Row Focus stands on a machine, not a place: an answer landing above it
    /// leaves it there, and nothing landing moves it from where the reader put
    /// it.
    #[test]
    fn no_answer_landing_moves_focus_from_where_the_reader_put_it() {
        let mut found = found(&[ONE, TWO]);
        found.answered(
            asking(1),
            listed(
                TWO,
                vec![
                    machine("aaaa0000", "a-box", true),
                    machine("bbbb0000", "b-box", true),
                ],
            ),
        );
        found.walk(true);
        assert_eq!(found.selected_fingerprint(), Some("bbbb0000"));

        found.answered(
            asking(1),
            listed(ONE, vec![machine("0000aaaa", "above", true)]),
        );
        assert_eq!(found.selected_fingerprint(), Some("bbbb0000"));
    }

    #[test]
    fn the_first_machine_found_takes_focus_only_until_the_reader_has_touched_anything() {
        let mut found = found(&[ONE, TWO]);
        found.focus_invite();
        found.answered(
            asking(1),
            listed(ONE, vec![machine("aaaa0000", "box", true)]),
        );
        assert_eq!(
            found.selected_fingerprint(),
            None,
            "the reader began on the Invite, and stays there"
        );

        let mut found = self::found(&[ONE, TWO]);
        found.answered(
            asking(1),
            listed(ONE, vec![machine("aaaa0000", "not-serving", false)]),
        );
        assert_eq!(found.selected_fingerprint(), Some("aaaa0000"));
        found.answered(
            asking(1),
            listed(TWO, vec![machine("bbbb0000", "serving", true)]),
        );
        assert_eq!(
            found.selected_fingerprint(),
            Some("aaaa0000"),
            "once it stands somewhere, it stays there"
        );
    }

    #[test]
    fn focus_walks_the_machines_and_the_invite_field_in_one_round() {
        let mut found = found(&[ONE]);
        found.answered(
            asking(1),
            listed(
                ONE,
                vec![
                    machine("aaaa0000", "a-box", true),
                    machine("bbbb0000", "b-box", true),
                ],
            ),
        );
        assert_eq!(found.selected_fingerprint(), Some("aaaa0000"));
        found.walk(true);
        assert_eq!(found.selected_fingerprint(), Some("bbbb0000"));
        found.walk(true);
        assert_eq!(found.selected_fingerprint(), None);
        found.walk(true);
        assert_eq!(found.selected_fingerprint(), Some("aaaa0000"));
        found.walk(false);
        assert_eq!(found.selected_fingerprint(), None);
        found.walk(false);
        assert_eq!(found.selected_fingerprint(), Some("bbbb0000"));
        found.focus_next();
        assert_eq!(found.selected_fingerprint(), None);
        found.focus_next();
        assert_eq!(found.selected_fingerprint(), Some("aaaa0000"));
    }

    #[test]
    fn an_answer_to_another_asking_lands_nowhere() {
        let mut found = found(&[ONE]);
        found.answered(
            asking(2),
            listed(ONE, vec![machine("aaaa0000", "late", true)]),
        );
        assert!(!found.any(), "from another asking");
        found.answered(
            asking(1),
            listed(TWO, vec![machine("aaaa0000", "stray", true)]),
        );
        assert!(!found.any(), "from a Relay this asking did not ask");

        found.answered(
            asking(1),
            listed(ONE, vec![machine("aaaa0000", "box", true)]),
        );
        found.answered(
            asking(1),
            listed(ONE, vec![machine("bbbb0000", "second", true)]),
        );
        assert_eq!(
            fingerprints(&found, 0),
            ["aaaa0000"],
            "a Relay answers an asking once"
        );

        found.end();
        found.answered(
            asking(1),
            listed(ONE, vec![machine("aaaa0000", "closed", true)]),
        );
        assert!(!found.any(), "once closed, nothing lands");
    }

    #[test]
    fn looking_again_keeps_what_was_found_and_focus_until_each_relay_answers() {
        let mut found = found(&[ONE]);
        found.answered(
            asking(1),
            listed(
                ONE,
                vec![
                    machine("aaaa0000", "a-box", true),
                    machine("bbbb0000", "b-box", true),
                ],
            ),
        );
        found.walk(true);
        found.look_again(asking(2));
        found.relays_known(asking(2), vec![ONE.to_owned()]);
        assert_eq!(fingerprints(&found, 0), ["aaaa0000", "bbbb0000"]);
        assert_eq!(found.selected_fingerprint(), Some("bbbb0000"));

        found.answered(
            asking(1),
            listed(ONE, vec![machine("cccc0000", "late", true)]),
        );
        assert_eq!(fingerprints(&found, 0), ["aaaa0000", "bbbb0000"]);

        found.answered(
            asking(2),
            listed(ONE, vec![machine("aaaa0000", "a-box", true)]),
        );
        assert_eq!(fingerprints(&found, 0), ["aaaa0000"]);
        assert_eq!(
            found.selected_fingerprint(),
            None,
            "a machine gone takes focus off it, to the Invite field"
        );
    }

    #[test]
    fn choosing_a_machine_says_what_to_do_there_and_nothing_more() {
        let mut found = found(&[ONE]);
        let mut paired = machine("bbbb0000", "host", false);
        paired.remote = Some("build-box".to_owned());
        found.answered(
            asking(1),
            listed(
                ONE,
                vec![
                    machine("aaaa0000", "workstation", true),
                    paired,
                    machine("cccc0000", "old-laptop", false),
                ],
            ),
        );
        assert!(found.choose());
        assert!(
            found.note().unwrap().contains("issue an Invite there"),
            "{:?}",
            found.note()
        );
        found.walk(true);
        assert_eq!(found.note(), None, "the next key takes it away");
        assert!(found.choose());
        assert!(found.note().unwrap().contains("Already paired"));
        found.walk(true);
        assert!(found.choose());
        assert!(found.note().unwrap().contains("run /serve there"));
        found.focus_invite();
        assert!(!found.choose(), "no machine stands to be chosen");
    }

    #[test]
    fn every_relay_says_how_it_stands_and_one_failing_stops_none_of_the_others() {
        let mut found = found(&[ONE, TWO]);
        assert_eq!(found.groups()[0].standing, GroupStanding::Asking);
        found.answered(
            asking(1),
            RelayMachines {
                address: ONE.to_owned(),
                answer: RelayMachinesAnswer::Unreachable {
                    message: "did not answer".to_owned(),
                },
            },
        );
        assert_eq!(found.groups()[1].standing, GroupStanding::Asking);
        found.answered(
            asking(1),
            listed(TWO, vec![machine("aaaa0000", "box", true)]),
        );
        assert_eq!(
            found.groups()[0].standing,
            GroupStanding::Unreachable("did not answer".to_owned())
        );
        assert_eq!(fingerprints(&found, 1), ["aaaa0000"]);
    }
}
