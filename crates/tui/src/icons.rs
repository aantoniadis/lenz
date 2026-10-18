//! Optional Nerd Font Mono decoration; action names always remain readable.
use std::borrow::Cow;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Icons { #[default] Plain, Nerd }
pub(crate) use lenz::icons::Icon;
impl Icons {
    /// `LENZ_ICONS`, else the settings file's `icons`, else plain.
    pub(crate) fn from_env() -> Self { Self::resolve(std::env::var("LENZ_ICONS").ok().as_deref(), lenz::settings::get().tools.icons) }
    pub(crate) fn resolve(env: Option<&str>, configured: Option<lenz::settings::Icons>) -> Self {
        match (env, configured) {
            (Some("nerd"), _) => Self::Nerd,
            (Some(_), _) => Self::Plain,
            (None, Some(lenz::settings::Icons::Nerd)) => Self::Nerd,
            _ => Self::Plain,
        }
    }
    pub(crate) fn toggle(&mut self) { *self=if *self==Self::Plain{Self::Nerd}else{Self::Plain}; }
    pub(crate) fn nerd(self)->bool { self==Self::Nerd }
    pub(crate) fn label(self,icon:Icon,text:&str)->Cow<'_,str> { icon.label(self.nerd(),text) }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn env_wins_then_settings_then_plain() {
        assert!(Icons::resolve(Some("nerd"), Some(lenz::settings::Icons::Plain)).nerd());
        assert!(Icons::resolve(None, Some(lenz::settings::Icons::Nerd)).nerd());
        assert!(!Icons::resolve(None, None).nerd());
        assert!(!Icons::resolve(Some("plain"), Some(lenz::settings::Icons::Nerd)).nerd());
    }
}
