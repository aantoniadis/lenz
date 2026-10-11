use super::{LoadedTheme, Palette, ThemeEntry, ThemeSelection, builtin_entries};
use std::sync::Arc;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeCommand {
    Select(ThemeSelection),
    OpenChooser,
    NativeAppearance(bool),
}
#[derive(Clone, Debug)]
pub struct ThemeSnapshot {
    pub selection: ThemeSelection,
    pub palette: Arc<Palette>,
    pub revision: u64,
    pub source: String,
}
#[derive(Default, Debug)]
pub struct ThemeEffect {
    pub repaint: bool,
    pub notice: Option<String>,
}
pub(super) struct ThemeController {
    pub selection: ThemeSelection,
    pub entries: Vec<ThemeEntry>,
    pub dark: bool,
    system: Option<LoadedTheme>,
    palette: Arc<Palette>,
    revision: u64,
    source: String,
}
impl ThemeController {
    pub fn new(
        selection: ThemeSelection,
        entries: Vec<ThemeEntry>,
        dark: bool,
        system: Option<LoadedTheme>,
    ) -> Self {
        let mut c = Self {
            selection,
            entries,
            dark,
            system,
            palette: Arc::new(super::DARCULA),
            revision: 0,
            source: String::new(),
        };
        c.resolve();
        c
    }
    pub fn snapshot(&self) -> ThemeSnapshot {
        ThemeSnapshot {
            selection: self.selection.clone(),
            palette: self.palette.clone(),
            revision: self.revision,
            source: self.source.clone(),
        }
    }
    /// The palette and its name that `selection` would show, without changing anything.
    pub fn resolved(&self, selection: &ThemeSelection) -> (Arc<Palette>, String) {
        let named = match selection {
            ThemeSelection::Named(id) => self.entries.iter().find(|e| &e.id == id),
            ThemeSelection::System => None,
        };
        if let Some(e) = named {
            (e.palette.clone(), e.label.clone())
        } else if let Some(t) = &self.system {
            (Arc::new(t.palette), t.name.clone())
        } else if self.dark {
            (Arc::new(super::DARCULA), "Darcula".into())
        } else {
            (Arc::new(super::LIGHT), "Light".into())
        }
    }
    fn resolve(&mut self) -> bool {
        let (palette, source) = self.resolved(&self.selection.clone());
        let changed = *self.palette != *palette;
        let source_changed = self.source != source;
        if changed {
            self.palette = palette;
            self.revision = self.revision.wrapping_add(1);
        }
        self.source = source;
        changed || source_changed
    }
    pub fn select(&mut self, selection: ThemeSelection) -> Result<bool, &'static str> {
        if let ThemeSelection::Named(id) = &selection
            && !self.entries.iter().any(|e| &e.id == id)
        {
            return Err("Theme no longer available");
        }
        let selected = self.selection != selection;
        self.selection = selection;
        Ok(self.resolve() || selected)
    }
    pub fn unavailable(&mut self, id: &str) {
        self.entries.retain(|e| e.id != id);
        self.resolve();
    }
    pub fn system(&mut self, theme: Option<LoadedTheme>) -> bool {
        self.system = theme;
        self.resolve()
    }
    pub fn native(&mut self, dark: bool) -> bool {
        self.dark = dark;
        self.resolve()
    }
    pub fn registry(&mut self, custom: Vec<ThemeEntry>) -> bool {
        self.entries = builtin_entries();
        self.entries.extend(custom);
        self.resolve()
    }
}
