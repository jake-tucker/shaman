//! What the composer's completion list tells about the slash command or
//! Skill its selection stands on, beyond the row that lists it: a slash
//! command's title, the other names it answers to and its keybinding, and a
//! Skill's name and scope, each above its description written out whole. A
//! wide dock draws it in a pane beside the list; a narrow one keeps each row
//! to a line of its own.

use ratatui::style::Modifier;

use crate::theme::Theme;

use super::{completion::CompletionRow, drawn_pane::DrawnPane};

/// The pane a wide completion dock details a row in, and the row of the pane
/// its description begins on.
pub(super) struct CompletionDetails<'a> {
    pub(super) pane: DrawnPane<'a>,
    description_row: usize,
}

impl<'a> CompletionDetails<'a> {
    /// The pane detailing `row`, `width` columns wide. Only a slash command
    /// or a Skill that can be chosen is detailed: the keys never stand on
    /// any other row.
    pub(super) fn of(row: CompletionRow<'a>, width: u16, theme: &Theme) -> Option<Self> {
        let heading = theme.text.primary.add_modifier(Modifier::BOLD);
        let mut pane = DrawnPane::default();
        let description = match row {
            CompletionRow::Command(command) => {
                pane.line(command.title, width, heading);
                let aliases = command.slash.map_or(&[][..], |slash| slash.aliases);
                if !aliases.is_empty() {
                    let label = if aliases.len() == 1 {
                        "Alias"
                    } else {
                        "Aliases"
                    };
                    let names = aliases
                        .iter()
                        .map(|alias| format!("/{alias}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    pane.line(&format!("{label} {names}"), width, theme.text.subdued);
                }
                if let Some(keybinding) = command.keybinding {
                    pane.line(
                        &format!("Key {}", keybinding.label),
                        width,
                        theme.text.subdued,
                    );
                }
                command.description
            }
            CompletionRow::Skill(skill) => {
                pane.line(&format!("${}", skill.name), width, heading);
                if let Some(scope) = &skill.scope {
                    pane.line(scope, width, theme.text.subdued);
                }
                skill.description.as_str()
            }
            CompletionRow::Insertion(_)
            | CompletionRow::StaleSkill(_)
            | CompletionRow::DisabledSkill(_)
            | CompletionRow::Message(_) => return None,
        };
        pane.blank();
        let description_row = pane.lines.len();
        pane.wrap(description, "", width, theme.text.primary);
        Some(Self {
            pane,
            description_row,
        })
    }

    /// The fewest rows the pane can be cut to and still say more than a
    /// one-line row would: through its description's first line, and the
    /// ellipsis beneath it where there is more.
    pub(super) fn least_rows(&self) -> usize {
        self.pane.lines.len().min(self.description_row + 2)
    }
}
