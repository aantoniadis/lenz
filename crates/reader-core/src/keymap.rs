//! Keymaps: the IntelliJ preset both adapters start with, an optional VS Code preset, and per-command overrides.
//! One resolution serves menu labels and key dispatch, so a rebinding never leaves a label claiming another key.
use std::collections::BTreeMap;

use crate::session::CommandId;

/// The keyboard language Lenz's chords follow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeymapPreset {
    #[default]
    IntelliJ,
    VsCode,
}

impl KeymapPreset {
    pub const ALL: [Self; 2] = [Self::IntelliJ, Self::VsCode];
    pub fn id(self) -> &'static str {
        match self {
            Self::IntelliJ => "intellij",
            Self::VsCode => "vscode",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::IntelliJ => "IntelliJ",
            Self::VsCode => "VS Code",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }
}

/// A key the adapter saw, with the modifiers that ran the binding. Adapters normalize their own spelling first
/// (shifted symbols arrive as their unshifted key, letters lowercase).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPress {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl KeyPress {
    /// `None` for a bare modifier press, which no binding answers.
    pub fn new(key: &str, ctrl: bool, shift: bool, alt: bool) -> Option<Self> {
        // Terminals and toolkits report Shift+ = `+`, Shift+` = `~`, Shift+\ = `|`, Shift+- = `_`, Shift+. = `>` and
        // Shift+[`/`]` as braces: they are the unshifted key with Shift.
        let (key, shift) = match key {
            "+" => ("=", true),
            "_" => ("-", true),
            "~" => ("`", true),
            "|" => ("\\", true),
            ">" => (".", true),
            "{" => ("[", true),
            "}" => ("]", true),
            _ => (key, shift),
        };
        if matches!(key, "" | "shift" | "control" | "ctrl" | "alt" | "option" | "super" | "meta" | "cmd" | "platform" | "fn") {
            return None;
        }
        Some(Self { key: key.to_ascii_lowercase(), ctrl, alt, shift })
    }
}

/// A shortcut: modifiers plus the unmodified key name (`ctrl+shift+k`, `f5`, `space`, `\\`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

impl Chord {
    /// `"Ctrl+Shift+K"`, `"F5"`, `"Alt+\\"`: modifiers in any order, then the key.
    pub fn parse(text: &str) -> Option<Self> {
        let mut tokens: Vec<&str> = text.split('+').map(str::trim).filter(|t| !t.is_empty()).collect();
        let key = tokens.pop()?.to_ascii_lowercase();
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        for token in tokens {
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "super" | "platform" => ctrl = true,
                "alt" | "option" => alt = true,
                "shift" => shift = true,
                _ => return None,
            }
        }
        // `F1`–`F99`: an `f` followed only by digits. `strip_prefix` keeps this off the slicing path.
        let function_key = key.strip_prefix('f').is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));
        if key.len() > 1 && !function_key && !matches!(key.as_str(), "space" | "tab" | "enter" | "escape" | "esc" | "backspace" | "delete" | "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown") {
            return None;
        }
        // A bare letter or symbol would run its command while the user types; Ctrl, Alt or a function key is required.
        if !ctrl && !alt && !key.starts_with('f') {
            return None;
        }
        Some(Self { ctrl, alt, shift, key })
    }

    /// The chord a remembered key press makes, if it may be one (`None` refuses a bare typing key).
    pub fn from_press(press: &KeyPress) -> Option<Self> {
        if !press.ctrl && !press.alt && !press.key.starts_with('f') {
            return None;
        }
        Some(Self { ctrl: press.ctrl, alt: press.alt, shift: press.shift, key: press.key.clone() })
    }

    pub fn matches(&self, press: &KeyPress) -> bool {
        self.ctrl == press.ctrl && self.alt == press.alt && self.shift == press.shift && self.key == press.key
    }

    /// How menus and dialogs spell it: `Ctrl+Alt+Shift+N`, `F5`, `Alt+\\`.
    pub fn label(&self) -> String {
        let mut out = String::new();
        for (on, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift")] {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(&display_key(&self.key));
        out
    }

    /// A chord a terminal cannot be relied on to deliver: flow control swallows Ctrl+S and Ctrl+Q, Ctrl+H/M/I
    /// arrive as Backspace/Enter/Tab, and Ctrl+Shift+letter needs the enhanced keyboard protocol. Such commands
    /// stay in the menus; the GUI delivers every chord.
    pub fn terminal_blocked(&self) -> bool {
        (matches!(self.key.as_str(), "q" | "s" | "h" | "m" | "i") && self.ctrl && !self.alt && !self.shift)
            || (self.ctrl && self.shift && self.key.len() == 1 && self.key.as_bytes()[0].is_ascii_alphabetic())
    }
}

fn display_key(key: &str) -> String {
    match key {
        "space" => "Space".into(),
        "pageup" => "Page Up".into(),
        "pagedown" => "Page Down".into(),
        "escape" | "esc" => "Esc".into(),
        _ if key.chars().count() == 1 => key.to_ascii_uppercase(),
        _ => {
            let mut chars = key.chars();
            chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        }
    }
}

/// A chord another command already runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError {
    TakenBy(CommandId),
    /// `Chord::parse` refused it: a bare typing key, or not a shortcut at all.
    Invalid,
}

type Preset = (CommandId, &'static [&'static str]);

/// The IntelliJ preset both adapters start with. `CloseAnnotations`, `CloseWindow` and every editing key the
/// adapters map as text input have no chord here; they stay in the command menus.
const INTELLIJ: &[Preset] = &[
    (CommandId::SearchEverywhere, &["Ctrl+K"]),
    (CommandId::FindFile, &["Ctrl+Shift+N", "Ctrl+P"]),
    (CommandId::FindSymbol, &["Ctrl+Alt+Shift+N"]),
    (CommandId::FindAction, &["Ctrl+Shift+A", "Ctrl+Shift+P"]),
    (CommandId::Open, &["Ctrl+O"]),
    (CommandId::SplitRight, &["Ctrl+\\"]),
    (CommandId::SplitDown, &["Ctrl+Shift+\\"]),
    (CommandId::Outline, &["Ctrl+Shift+O"]),
    (CommandId::FindInFile, &["Ctrl+F"]),
    (CommandId::CompareSelected, &["Ctrl+D"]),
    (CommandId::CompareSaved, &["Ctrl+Shift+D"]),
    (CommandId::SearchProject, &["Ctrl+Shift+F"]),
    (CommandId::GoToLine, &["Ctrl+G"]),
    (CommandId::Reload, &["Ctrl+R", "F5"]),
    (CommandId::SoftWrap, &["Alt+Z"]),
    (CommandId::LanguageServer, &["F2"]),
    (CommandId::StatusDetails, &["Shift+F2"]),
    (CommandId::Definition, &["F12"]),
    (CommandId::PeekDefinition, &["Alt+F12"]),
    (CommandId::TypeDefinition, &["Ctrl+Shift+B"]),
    (CommandId::Implementation, &["Ctrl+Alt+B"]),
    (CommandId::AnnotateBlame, &["Alt+B"]),
    (CommandId::Usages, &["Shift+F12"]),
    (CommandId::CallHierarchy, &["Ctrl+Alt+H"]),
    // Ctrl+H is IntelliJ's Type Hierarchy; Alt+H is the route terminals can always deliver.
    (CommandId::TypeHierarchy, &["Ctrl+H", "Alt+H"]),
    (CommandId::Documentation, &["Ctrl+Q"]),
    (CommandId::Completion, &["Ctrl+Space"]),
    (CommandId::Save, &["Ctrl+S"]),
    (CommandId::SaveAs, &["Ctrl+Shift+S"]),
    (CommandId::NewDocument, &["Ctrl+N"]),
    (CommandId::Close, &["Ctrl+F4"]),
    (CommandId::ExtendSelection, &["Ctrl+W"]),
    (CommandId::ShrinkSelection, &["Ctrl+Shift+W"]),
    // Several cursors: IntelliJ's Alt+J selects the next occurrence; IntelliJ has no add-cursor chord, so the two
    // cursor commands take the arrows their adapters have always used, and stay rebindable.
    (CommandId::AddCursorBelow, &["Alt+Shift+Down"]),
    (CommandId::AddCursorAbove, &["Alt+Shift+Up"]),
    (CommandId::SelectNextOccurrence, &["Alt+J"]),
    (CommandId::FocusBreadcrumbs, &["Ctrl+Shift+."]),
    (CommandId::ToggleBookmark, &["F11"]),
    (CommandId::SetBookmarkMnemonic, &["Ctrl+F11"]),
    (CommandId::ShowBookmarks, &["Shift+F11"]),
    (CommandId::RecentFiles, &["Ctrl+E"]),
    (CommandId::RecentLocations, &["Ctrl+Alt+E"]),
    (CommandId::Collapse, &["Ctrl+-"]),
    (CommandId::Expand, &["Ctrl+="]),
    (CommandId::CollapseAll, &["Ctrl+Shift+-"]),
    (CommandId::ExpandAll, &["Ctrl+Shift+="]),
    (CommandId::QuitAll, &["Ctrl+Shift+Q"]),
];

/// VS Code's chords where VS Code has the same action. Actions VS Code leaves unbound (outline, documentation,
/// language-server status, collapse all) have no chord there and stay in the menus; Ctrl+H is VS Code's Replace,
/// which Lenz offers from its find box and menus rather than a global chord (an empty replacement would delete the
/// current match), and F2 its Rename, so Type Hierarchy and the server status move off them.
const VS_CODE: &[Preset] = &[
    (CommandId::SearchEverywhere, &["Ctrl+T"]),
    (CommandId::FindFile, &["Ctrl+P"]),
    (CommandId::FindSymbol, &["Ctrl+Shift+O"]),
    (CommandId::FindAction, &["Ctrl+Shift+P"]),
    (CommandId::Open, &["Ctrl+O"]),
    (CommandId::SplitRight, &["Ctrl+\\"]),
    (CommandId::SplitDown, &["Ctrl+Shift+\\"]),
    (CommandId::FindInFile, &["Ctrl+F"]),
    (CommandId::CompareSelected, &["Ctrl+D"]),
    (CommandId::CompareSaved, &["Ctrl+Shift+D"]),
    (CommandId::SearchProject, &["Ctrl+Shift+F"]),
    (CommandId::GoToLine, &["Ctrl+G"]),
    (CommandId::Reload, &["F5"]),
    (CommandId::SoftWrap, &["Alt+Z"]),
    (CommandId::StatusDetails, &["Shift+F2"]),
    (CommandId::Definition, &["F12"]),
    (CommandId::PeekDefinition, &["Alt+F12"]),
    (CommandId::TypeDefinition, &["Ctrl+Shift+B"]),
    (CommandId::Implementation, &["Ctrl+F12"]),
    (CommandId::AnnotateBlame, &["Alt+B"]),
    (CommandId::Usages, &["Shift+F12"]),
    (CommandId::CallHierarchy, &["Alt+Shift+H"]),
    (CommandId::TypeHierarchy, &["Alt+H"]),
    (CommandId::Completion, &["Ctrl+Space"]),
    (CommandId::Save, &["Ctrl+S"]),
    (CommandId::SaveAs, &["Ctrl+Shift+S"]),
    (CommandId::NewDocument, &["Ctrl+N"]),
    (CommandId::Close, &["Ctrl+W"]),
    (CommandId::ExtendSelection, &["Alt+Shift+Right"]),
    (CommandId::ShrinkSelection, &["Alt+Shift+Left"]),
    // VS Code's own add-cursor chords; Alt+Shift+Down/Up stay as the arrows both adapters have always delivered.
    // VS Code's Ctrl+D (select next occurrence) is Compare Selected here, so Alt+J keeps that command.
    (CommandId::AddCursorBelow, &["Ctrl+Alt+Down", "Alt+Shift+Down"]),
    (CommandId::AddCursorAbove, &["Ctrl+Alt+Up", "Alt+Shift+Up"]),
    (CommandId::SelectNextOccurrence, &["Alt+J"]),
    (CommandId::FocusBreadcrumbs, &["Ctrl+Shift+."]),
    (CommandId::ToggleBookmark, &["F11"]),
    (CommandId::SetBookmarkMnemonic, &["Ctrl+F11"]),
    (CommandId::ShowBookmarks, &["Shift+F11"]),
    (CommandId::RecentFiles, &["Ctrl+E"]),
    (CommandId::RecentLocations, &["Ctrl+Alt+E"]),
    (CommandId::Collapse, &["Ctrl+Shift+["]),
    (CommandId::Expand, &["Ctrl+Shift+]"]),
    (CommandId::QuitAll, &["Ctrl+Shift+Q"]),
];

fn preset_chords(preset: KeymapPreset) -> &'static [Preset] {
    match preset {
        KeymapPreset::IntelliJ => INTELLIJ,
        KeymapPreset::VsCode => VS_CODE,
    }
}

/// The resolved chords of one keymap: presets with the overrides applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keymap {
    preset: KeymapPreset,
    overrides: BTreeMap<CommandId, Option<Chord>>,
    bindings: Vec<(Chord, CommandId)>,
    /// Preset chords an override took from their command, reported when the override is made.
    lost: Vec<(CommandId, Chord, CommandId)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new(KeymapPreset::default(), BTreeMap::new())
    }
}

impl Keymap {
    pub fn new(preset: KeymapPreset, overrides: BTreeMap<CommandId, Option<Chord>>) -> Self {
        let mut bindings: Vec<(Chord, CommandId)> = Vec::new();
        let mut lost = Vec::new();
        // An explicit override is the user's decision and always resolves; a second override of the same chord stays
        // with the first command, which is what the dialog's refusal prevents.
        for (&id, chord) in &overrides {
            let Some(chord) = chord else { continue };
            match bindings.iter().find(|(c, _)| c == chord) {
                Some((_, winner)) => lost.push((id, chord.clone(), *winner)),
                None => bindings.push((chord.clone(), id)),
            }
        }
        for &(id, chords) in preset_chords(preset) {
            if overrides.contains_key(&id) {
                continue;
            }
            for text in chords {
                let Some(chord) = Chord::parse(text) else { continue };
                match bindings.iter().find(|(c, _)| *c == chord) {
                    Some((_, winner)) => lost.push((id, chord, *winner)),
                    None => bindings.push((chord, id)),
                }
            }
        }
        Self { preset, overrides, bindings, lost }
    }

    pub fn preset(&self) -> KeymapPreset {
        self.preset
    }

    pub fn overrides(&self) -> &BTreeMap<CommandId, Option<Chord>> {
        &self.overrides
    }

    /// The command `press` runs, if any.
    pub fn command(&self, press: &KeyPress) -> Option<CommandId> {
        self.bindings.iter().find(|(chord, _)| chord.matches(press)).map(|(_, id)| *id)
    }

    /// Every chord that runs `id` now, in preset order.
    pub fn chords(&self, id: CommandId) -> Vec<&Chord> {
        self.bindings.iter().filter(|(_, bound)| *bound == id).map(|(chord, _)| chord).collect()
    }

    /// The chord menus show for `id`: its first one.
    pub fn hint(&self, id: CommandId) -> Option<String> {
        self.chords(id).first().map(|chord| chord.label())
    }

    /// Every chord for `id`, joined for a menu line.
    pub fn labels(&self, id: CommandId) -> Option<String> {
        let chords = self.chords(id);
        (!chords.is_empty()).then(|| chords.iter().map(|c| c.label()).collect::<Vec<_>>().join(", "))
    }

    pub fn overridden(&self, id: CommandId) -> bool {
        self.overrides.contains_key(&id)
    }

    /// A preset chord `id` lost to an override: the chord and the command that now runs it.
    pub fn shadowed(&self, id: CommandId) -> Vec<(&Chord, CommandId)> {
        self.lost.iter().filter(|(loser, _, _)| *loser == id).map(|(_, chord, winner)| (chord, *winner)).collect()
    }

    /// Who runs `chord` now.
    pub fn owner(&self, chord: &Chord) -> Option<CommandId> {
        self.bindings.iter().find(|(bound, _)| bound == chord).map(|(_, id)| *id)
    }

    /// Rebind one command. A chord another command already runs is refused, so no action silently loses its key.
    pub fn bind(&self, id: CommandId, chord: Chord) -> Result<Self, BindError> {
        if let Some(owner) = self.owner(&chord).filter(|owner| *owner != id) {
            return Err(BindError::TakenBy(owner));
        }
        let mut overrides = self.overrides.clone();
        overrides.insert(id, Some(chord));
        Ok(Self::new(self.preset, overrides))
    }

    /// Forget `id`'s override, returning it to the preset.
    pub fn reset(&self, id: CommandId) -> Self {
        let mut overrides = self.overrides.clone();
        overrides.remove(&id);
        Self::new(self.preset, overrides)
    }

    /// Keep the overrides and change the preset.
    pub fn with_preset(&self, preset: KeymapPreset) -> Self {
        Self::new(preset, self.overrides.clone())
    }

    /// Persistable overrides: command slug → chord label, or `none` for a command the user unbound.
    pub fn overrides_for_settings(&self) -> BTreeMap<String, String> {
        self.overrides.iter().map(|(id, chord)| (slug(*id), chord.as_ref().map_or_else(|| "none".into(), Chord::label))).collect()
    }
}

/// One line of the keymap dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeymapRow {
    pub command: CommandId,
    pub label: &'static str,
    /// Its chords now, or `no shortcut`.
    pub chords: String,
    /// `your choice` for an override, and what a displaced binding runs instead.
    pub note: Option<String>,
}

impl Keymap {
    /// Every command a keymap can move: the ones with a chord now and the ones the user unbound, grouped like the
    /// menus.
    pub fn rows(&self) -> Vec<KeymapRow> {
        use crate::actions::CommandGroup;
        let mut rows: Vec<KeymapRow> = CommandId::ALL
            .into_iter()
            .filter(|id| !self.chords(*id).is_empty() || self.overridden(*id))
            .map(|id| {
                let chords = self.labels(id).unwrap_or_else(|| "no shortcut".into());
                let mut notes = Vec::new();
                if self.overridden(id) {
                    notes.push("your choice".to_string());
                }
                for (chord, winner) in self.shadowed(id) {
                    notes.push(format!("{} now runs {}", chord.label(), winner.label()));
                }
                KeymapRow { command: id, label: id.label(), chords, note: (!notes.is_empty()).then(|| notes.join("; ")) }
            })
            .collect();
        let order = |id: CommandId| CommandGroup::ALL.iter().position(|group| *group == id.group()).unwrap_or(usize::MAX);
        rows.sort_by(|a, b| order(a.command).cmp(&order(b.command)).then(a.label.cmp(b.label)));
        rows
    }
}

/// A command's stable settings key, derived from its variant name (`SearchEverywhere` → `search_everywhere`).
pub fn slug(id: CommandId) -> String {
    let name = format!("{id:?}");
    let mut out = String::with_capacity(name.len() + 8);
    for (i, c) in name.char_indices() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// The command a settings key names, if any.
pub fn from_slug(slug: &str) -> Option<CommandId> {
    CommandId::ALL.into_iter().find(|id| self::slug(*id) == slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(text: &str) -> KeyPress {
        let chord = Chord::parse(text).unwrap();
        KeyPress::new(&chord.key, chord.ctrl, chord.shift, chord.alt).unwrap()
    }

    #[test]
    fn every_preset_chord_parses_and_is_bound_once() {
        for preset in KeymapPreset::ALL {
            let keymap = Keymap::new(preset, BTreeMap::new());
            assert!(keymap.lost.is_empty(), "{preset:?} has conflicting preset chords: {:?}", keymap.lost);
            for &(id, chords) in preset_chords(preset) {
                for text in chords {
                    let chord = Chord::parse(text).unwrap_or_else(|| panic!("{preset:?} {id:?}: {text}"));
                    assert_eq!(keymap.command(&press(text)), Some(id), "{preset:?} {text}");
                    assert_eq!(chord.label(), *text, "a label round-trips");
                }
            }
        }
    }

    #[test]
    fn intellij_is_the_default_and_keeps_the_documented_chords() {
        let keymap = Keymap::default();
        assert_eq!(keymap.preset(), KeymapPreset::IntelliJ);
        assert_eq!(keymap.hint(CommandId::SearchEverywhere).as_deref(), Some("Ctrl+K"));
        assert_eq!(keymap.labels(CommandId::FindFile).as_deref(), Some("Ctrl+Shift+N, Ctrl+P"));
        assert_eq!(keymap.labels(CommandId::Reload).as_deref(), Some("Ctrl+R, F5"));
        assert_eq!(keymap.labels(CommandId::TypeHierarchy).as_deref(), Some("Ctrl+H, Alt+H"));
        assert_eq!(keymap.hint(CommandId::Documentation).as_deref(), Some("Ctrl+Q"));
        assert_eq!(keymap.hint(CommandId::CloseWindow), None, "the terminal's view-close key is not the GUI's");
        assert_eq!(keymap.hint(CommandId::CloseAnnotations), None, "Alt+B toggles the annotations it closes");
        assert_eq!(keymap.hint(CommandId::StatusDetails).as_deref(), Some("Shift+F2"));
        assert_eq!(keymap.hint(CommandId::LanguageServer).as_deref(), Some("F2"));
    }

    #[test]
    fn vs_code_swaps_the_chords_it_shares_with_vs_code_and_leaves_the_rest_to_menus() {
        let keymap = Keymap::new(KeymapPreset::VsCode, BTreeMap::new());
        assert_eq!(keymap.hint(CommandId::FindAction).as_deref(), Some("Ctrl+Shift+P"));
        assert_eq!(keymap.hint(CommandId::FindFile).as_deref(), Some("Ctrl+P"));
        assert_eq!(keymap.hint(CommandId::SearchEverywhere).as_deref(), Some("Ctrl+T"));
        assert_eq!(keymap.hint(CommandId::FindSymbol).as_deref(), Some("Ctrl+Shift+O"));
        assert_eq!(keymap.hint(CommandId::Close).as_deref(), Some("Ctrl+W"));
        assert_eq!(keymap.hint(CommandId::ExtendSelection).as_deref(), Some("Alt+Shift+Right"));
        assert_eq!(keymap.hint(CommandId::TypeHierarchy).as_deref(), Some("Alt+H"), "Ctrl+H is Replace in VS Code");
        assert_eq!(keymap.hint(CommandId::Implementation).as_deref(), Some("Ctrl+F12"));
        // VS Code's F2 renames and Ctrl+Shift+O finds symbols: neither leaves a stale chord behind.
        for id in [CommandId::LanguageServer, CommandId::Documentation, CommandId::Outline, CommandId::CollapseAll, CommandId::ExpandAll] {
            assert_eq!(keymap.hint(id), None, "{id:?}");
        }
        // The swapped key now runs its new command, not the IntelliJ one.
        assert_eq!(keymap.command(&press("Ctrl+W")), Some(CommandId::Close));
        assert_eq!(keymap.command(&press("Ctrl+H")), None);
    }

    #[test]
    fn an_override_replaces_the_command_chords_and_names_what_it_displaced() {
        let keymap = Keymap::default().bind(CommandId::Save, Chord::parse("Ctrl+Alt+S").unwrap()).unwrap();
        assert_eq!(keymap.hint(CommandId::Save).as_deref(), Some("Ctrl+Alt+S"));
        assert_eq!(keymap.labels(CommandId::Reload).as_deref(), Some("Ctrl+R, F5"), "an alias keeps the command reachable");
        assert!(keymap.overridden(CommandId::Save));
        assert!(keymap.shadowed(CommandId::Save).is_empty());

        // A preset change re-resolves: Ctrl+T is free in IntelliJ but VS Code's Search Everywhere, which loses it.
        let shadow = Keymap::default().bind(CommandId::ShowCommit, Chord::parse("Ctrl+T").unwrap()).unwrap();
        let vscode = shadow.with_preset(KeymapPreset::VsCode);
        assert_eq!(vscode.command(&press("Ctrl+T")), Some(CommandId::ShowCommit));
        assert_eq!(vscode.shadowed(CommandId::SearchEverywhere), [(&Chord::parse("Ctrl+T").unwrap(), CommandId::ShowCommit)]);
        assert_eq!(vscode.hint(CommandId::SearchEverywhere), None, "a displaced command keeps its menu line");
        // Resetting the override returns the preset's chord.
        let reset = vscode.reset(CommandId::ShowCommit);
        assert_eq!(reset.hint(CommandId::SearchEverywhere).as_deref(), Some("Ctrl+T"));
        assert!(reset.overrides().is_empty());
    }

    #[test]
    fn a_hand_edited_duplicate_override_resolves_deterministically_and_reports_the_loser() {
        let overrides = BTreeMap::from([
            (CommandId::Save, Some(Chord::parse("Ctrl+K").unwrap())),
            (CommandId::ToggleBookmark, Some(Chord::parse("Ctrl+K").unwrap())),
        ]);
        let keymap = Keymap::new(KeymapPreset::IntelliJ, overrides);
        // Overrides resolve in command order; the earlier command keeps the chord.
        let winner = keymap.command(&press("Ctrl+K"));
        assert!(matches!(winner, Some(CommandId::Save | CommandId::ToggleBookmark)), "{winner:?}");
        let loser = if winner == Some(CommandId::Save) { CommandId::ToggleBookmark } else { CommandId::Save };
        assert_eq!(keymap.shadowed(loser).len(), 1, "the losing override is reported");
        assert!(keymap.shadowed(CommandId::SearchEverywhere).iter().any(|(_, winner)| *winner != CommandId::SearchEverywhere));
    }

    #[test]
    fn a_chord_another_command_runs_is_refused_rather_than_silently_hiding_it() {
        let keymap = Keymap::default();
        assert_eq!(keymap.bind(CommandId::Save, Chord::parse("Ctrl+K").unwrap()), Err(BindError::TakenBy(CommandId::SearchEverywhere)));
        assert_eq!(keymap.bind(CommandId::Save, Chord::parse("F5").unwrap()), Err(BindError::TakenBy(CommandId::Reload)));
        // The command's own chord is no refusal.
        assert!(keymap.bind(CommandId::Reload, Chord::parse("Ctrl+R").unwrap()).is_ok());
        assert_eq!(Chord::parse("k"), None, "a bare typing key is not a shortcut");
        assert_eq!(Chord::parse("Ctrl+"), None);
        assert_eq!(Chord::parse("Shift+Alt+Right").map(|c| c.label()).as_deref(), Some("Alt+Shift+Right"));
    }

    #[test]
    fn shifted_symbols_normalize_into_the_unshifted_key() {
        assert_eq!(KeyPress::new("+", true, false, false), Some(KeyPress { key: "=".into(), ctrl: true, alt: false, shift: true }));
        assert_eq!(KeyPress::new("_", true, true, false), Some(KeyPress { key: "-".into(), ctrl: true, alt: false, shift: true }));
        assert_eq!(KeyPress::new("|", true, false, false), Some(KeyPress { key: "\\".into(), ctrl: true, alt: false, shift: true }));
        assert_eq!(KeyPress::new("K", true, false, false), Some(KeyPress { key: "k".into(), ctrl: true, alt: false, shift: false }));
        assert_eq!(KeyPress::new("shift", false, true, false), None);
        let keymap = Keymap::default();
        assert_eq!(keymap.command(&KeyPress::new("+", true, false, false).unwrap()), Some(CommandId::ExpandAll));
        assert_eq!(keymap.command(&KeyPress::new("_", true, false, false).unwrap()), Some(CommandId::CollapseAll));
        assert_eq!(keymap.command(&KeyPress::new("|", true, true, false).unwrap()), Some(CommandId::SplitDown));
    }

    #[test]
    fn overrides_kept_through_a_preset_change_and_saved_by_slug() {
        let keymap = Keymap::new(KeymapPreset::VsCode, BTreeMap::new()).bind(CommandId::Save, Chord::parse("Ctrl+Alt+S").unwrap()).unwrap();
        let intellij = keymap.with_preset(KeymapPreset::IntelliJ);
        assert_eq!(intellij.preset(), KeymapPreset::IntelliJ);
        assert_eq!(intellij.hint(CommandId::Save).as_deref(), Some("Ctrl+Alt+S"), "an override outlives the preset");
        assert_eq!(intellij.hint(CommandId::Close).as_deref(), Some("Ctrl+F4"));
        assert_eq!(intellij.overrides_for_settings(), BTreeMap::from([("save".to_string(), "Ctrl+Alt+S".to_string())]));
    }

    #[test]
    fn every_command_has_a_distinct_settings_slug() {
        let mut seen = std::collections::BTreeSet::new();
        for id in CommandId::ALL {
            let slug = slug(id);
            assert!(slug.chars().all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()), "{id:?}: {slug}");
            assert!(seen.insert(slug.clone()), "duplicate slug {slug}");
            assert_eq!(from_slug(&slug), Some(id));
        }
    }

    #[test]
    fn terminal_undeliverable_chords_are_known_centrally() {
        let keymap = Keymap::default();
        let blocked = |id| keymap.chords(id).iter().any(|c| c.terminal_blocked());
        assert!(blocked(CommandId::Documentation), "Ctrl+Q is flow control and closes the view");
        assert!(blocked(CommandId::Save), "Ctrl+S is flow control");
        assert!(blocked(CommandId::TypeHierarchy), "Ctrl+H arrives as Backspace");
        assert!(!keymap.chords(CommandId::Reload).iter().all(|c| c.terminal_blocked()), "F5 and Ctrl+R are delivered");
        assert!(!keymap.chords(CommandId::FindFile).iter().all(|c| c.terminal_blocked()));
    }
}
