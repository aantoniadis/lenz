use xkbcommon::xkb as xkbc;

// Cancelled compose also covers unmatched printable keys, whose accent fallback is intentional.
// Escape cancels instead: never send that fallback as a document edit first.
pub(super) fn discard_cancelled_compose(
    keysym: xkbc::Keysym,
    state: &mut xkbc::compose::State,
    preedit: &mut Option<String>,
) -> bool {
    if keysym != xkbc::Keysym::Escape {
        return false;
    }
    state.reset();
    *preedit = None;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xkb_escape_discards_composition_and_the_next_phase_still_composes() {
        let context = xkbc::Context::new(xkbc::CONTEXT_NO_FLAGS);
        let table = xkbc::compose::Table::new_from_locale(
            &context,
            std::ffi::OsStr::new("C"),
            xkbc::compose::COMPILE_NO_FLAGS,
        )
        .expect("local XKB compose table");
        let mut state = xkbc::compose::State::new(&table, xkbc::compose::STATE_NO_FLAGS);
        state.feed(xkbc::Keysym::dead_acute);
        assert_eq!(state.status(), xkbc::Status::Composing);
        let mut preedit = Some("´".to_string());
        state.feed(xkbc::Keysym::Escape);
        assert_eq!(state.status(), xkbc::Status::Cancelled);
        assert!(discard_cancelled_compose(xkbc::Keysym::Escape, &mut state, &mut preedit));
        assert_eq!(preedit, None, "cancel must not supply an accent for commit");
        assert_eq!(state.status(), xkbc::Status::Nothing);
        state.feed(xkbc::Keysym::dead_acute);
        state.feed(xkbc::Keysym::e);
        assert_eq!(state.status(), xkbc::Status::Composed);
        assert_eq!(state.utf8().as_deref(), Some("é"));

        state.reset();
        state.feed(xkbc::Keysym::dead_acute);
        state.feed(xkbc::Keysym::q);
        assert_eq!(state.status(), xkbc::Status::Cancelled);
        let mut preedit = Some("´".to_string());
        let original = preedit.clone();
        assert!(!discard_cancelled_compose(xkbc::Keysym::q, &mut state, &mut preedit));
        assert_eq!(preedit, original, "unmatched printable fallback is preserved");
    }
}
