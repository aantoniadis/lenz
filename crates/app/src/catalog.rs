//! Every command the palette lists, with its label, group, shortcut and current value. Both frontends render these
//! and dispatch `Action` with an exhaustive match; disabled reasons come from the existing core checks via `Availability`.
use std::borrow::Cow;
use lenz_engine::{HighlightLevel, Options, WhitespaceMode};
use crate::commands::{self, Command, DiffLayout, Interaction};
use crate::edit_session::EditMenuCommand;
use crate::file_actions::FileAction;
use crate::layout::LayoutCommand;
use crate::markdown::{MarkdownLayout, PresentationMode};
use crate::merge::Pick;
use crate::merge_session::Act;
use crate::pr_session::DetailsTab;
use crate::version_session::PickAct;
use crate::view::ScrollMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepoAction {
    Refresh, Branches, StagedChanges, CopyUrl, OpenUrl, Back, Forward,
    /// Edit the history filter (messages and paths).
    Filter, LocalChanges, AllBranches, CurrentBranch,
    /// The sidebar's selected branch against the current one: from their merge base, or tip to tip.
    CompareBranch { merge_base: bool },
    /// The selected commit against the sidebar's selected branch.
    CompareCommit,
    NextParent, FileHistory, WriteGraph, CopyHash, CopySubject, CopyDetails, ShowInGraph,
    /// The sidebar's selected worktree in a view (or window) of its own.
    OpenWorktree,
    /// Collapse or expand the commit message panel in Focus.
    ToggleMessage,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrAction { Details(DetailsTab), /** Back from a details tab to the diff. */ Diff, MarkViewedNext, Refresh, OutdatedThreads, Threads, CheckHead }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HelpAction { Shortcuts, DiffOptions, Settings }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Comparison(Command), Interaction(Interaction), Edit(EditMenuCommand), File(FileAction),
    /// Index into `image_session::image_commands()`.
    Image(usize),
    Repository(RepoAction), Pr(PrAction), Merge(Act), Pick(PickAct), Layout(LayoutCommand), Help(HelpAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group { Navigate, Search, Edit, Copy, File, View, Diff, Repository, PullRequest, Merge, Help }
impl Group {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Navigate => "Navigate", Self::Search => "Search", Self::Edit => "Edit", Self::Copy => "Copy", Self::File => "File",
            Self::View => "View", Self::Diff => "Diff", Self::Repository => "Repository", Self::PullRequest => "Pull request",
            Self::Merge => "Merge", Self::Help => "Help",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry { pub action: Action, pub label: Cow<'static, str>, pub group: Group, pub shortcut: Option<&'static str>, pub current: bool }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface { Comparison, Merge, VersionPicker }

/// What the window shows now; decides which entries exist and which values are current.
#[derive(Clone, Copy, Debug)]
pub struct Scope {
    pub surface: Surface, pub repository: bool, pub pr: bool, pub image: bool, pub markdown: bool,
    pub focused: bool,
    /// The pull request details tab on screen (`None`: the diff).
    pub pr_tab: Option<DetailsTab>,
    /// The comparison shows one column (Unified).
    pub unified: bool,
    pub options: Options, pub scroll: ScrollMode, pub presentation: PresentationMode, pub context_hidden: bool, pub blame: bool,
    pub image_mode: crate::image_session::Mode, pub markdown_layout: MarkdownLayout,
    /// The find options while a find or Change Set search input is open (`None`: none is).
    pub find: Option<lenz_content::find::FindOptions>,
}
impl Scope {
    /// A plain text comparison with default options.
    pub fn comparison() -> Self {
        Self { surface: Surface::Comparison, repository: false, pr: false, focused: false, pr_tab: None, unified: false, image: false, markdown: false, options: Options::default(),
               scroll: ScrollMode::Shared, presentation: PresentationMode::Source, context_hidden: true, blame: false,
               image_mode: crate::image_session::Mode::TwoUp, markdown_layout: MarkdownLayout::Inline, find: None }
    }
}

/// Why an action cannot run now; adapters answer from the existing core checks.
pub trait Availability { fn disabled(&self, action: Action) -> Option<Cow<'static, str>>; }

pub struct OptionRow { pub label: &'static str, pub choices: Vec<Entry> }

const WHITESPACE: [WhitespaceMode; 4] = [WhitespaceMode::Default, WhitespaceMode::Trim, WhitespaceMode::Ignore, WhitespaceMode::IgnoreTrimChunks];
const HIGHLIGHT: [HighlightLevel; 3] = [HighlightLevel::Word, HighlightLevel::Char, HighlightLevel::Line];
const SCROLL: [ScrollMode; 4] = [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded, ScrollMode::Independent];

fn entry(action: Action, label: impl Into<Cow<'static, str>>, group: Group, shortcut: Option<&'static str>) -> Entry {
    Entry { action, label: label.into(), group, shortcut, current: false }
}
fn choice(action: Action, label: String, group: Group, current: bool) -> Entry { Entry { current, ..entry(action, label, group, None) } }

/// A shared lower-case mode name as a palette value: "trim" → "Trim".
pub(crate) fn title(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

fn settings(scope: &Scope) -> [Vec<Entry>; 3] {
    [
        WHITESPACE.map(|m| choice(Action::Comparison(Command::SetWhitespace(m)), format!("Whitespace: {}", title(commands::whitespace_name(m))), Group::Diff, m == scope.options.whitespace)).into(),
        HIGHLIGHT.map(|l| choice(Action::Comparison(Command::SetHighlight(l)), format!("Highlight: {}", title(commands::highlight_name(l))), Group::Diff, l == scope.options.highlight)).into(),
        SCROLL.map(|s| choice(Action::Comparison(Command::SetScroll(s)), format!("Scroll: {}", title(commands::scroll_name(s))), Group::Diff, s == scope.scroll)).into(),
    ]
}

/// The Diff popover: one row per setting, then the toggles.
pub fn diff_options(scope: &Scope) -> Vec<OptionRow> {
    if scope.surface != Surface::Comparison || scope.image { return Vec::new(); }
    let [whitespace, highlight, scroll] = settings(scope);
    let toggle = |action, label: &str, on| choice(action, label.to_string(), Group::Diff, on);
    vec![
        OptionRow { label: "Layout", choices: diff_layout_menu(scope) },
        OptionRow { label: "Whitespace", choices: whitespace },
        OptionRow { label: "Highlight", choices: highlight },
        OptionRow { label: "Scroll", choices: scroll },
        OptionRow { label: "Show", choices: vec![toggle(Action::Comparison(Command::ToggleContext), "Only changes", scope.context_hidden)] },
        OptionRow { label: "Compare", choices: vec![
            toggle(Action::Comparison(Command::ToggleCase), "Ignore case", scope.options.ignore_case),
            toggle(Action::Interaction(Interaction::Blame), "Blame", scope.blame),
        ] },
    ]
}

pub fn entries(scope: &Scope) -> Vec<Entry> {
    use Group as G;
    let c = |command: Command, group, key| entry(Action::Comparison(command), command.label(), group, key);
    let i = |command: Interaction, group, key| entry(Action::Interaction(command), command.label(), group, key);
    let l = |command: LayoutCommand, key| entry(Action::Layout(command), command.label(), G::View, key);
    let mut out = Vec::new();
    match scope.surface {
        Surface::Merge => {
            let m = |act: Act, group| entry(Action::Merge(act), act.label(), group, None);
            out.extend([Act::PrevChange, Act::NextChange, Act::PrevConflict, Act::NextConflict].map(|a| m(a, G::Navigate)));
            out.extend([Act::Find, Act::NextMatch, Act::PreviousMatch, Act::NextPane, Act::ClearFind].map(|a| m(a, G::Search)));
            let o = scope.find.unwrap_or_default();
            out.extend([(Interaction::FindCase, "alt-c", o.case), (Interaction::FindWord, "alt-w", o.word), (Interaction::FindRegex, "alt-r", o.regex)]
                .map(|(command, key, on)| Entry { current: on, ..i(command, G::Search, Some(key)) }));
            out.extend([Act::Undo, Act::Redo, Act::Save, Act::Done, Act::Cancel, Act::Reload].map(|a| m(a, G::Edit)));
            out.extend([Act::ResolveSimple, Act::ApplyNonConflicting, Act::AllFrom(Pick::Ours), Act::AllFrom(Pick::Theirs), Act::StartOver, Act::MarkResolved].map(|a| m(a, G::Merge)));
            out.extend([Act::ToggleBase, Act::Whitespace, Act::Scroll].map(|a| m(a, G::View)));
            out.push(i(Interaction::ResetPaneSizes, G::View, None));
        }
        Surface::Comparison | Surface::VersionPicker => {
            out.extend([
                c(Command::PreviousChange, G::Navigate, Some("p")), c(Command::NextChange, G::Navigate, Some("n")),
                // with Ctrl: plain [ and ] are search text while a list has the keyboard
                c(Command::PreviousFile, G::Navigate, Some("ctrl-[")), c(Command::NextFile, G::Navigate, Some("ctrl-]")),
                c(Command::ToggleFold, G::Navigate, Some("x")), c(Command::UnfoldAll, G::Navigate, None),
                i(Interaction::GoToLine, G::Navigate, Some("ctrl-g")), i(Interaction::ScrollLeft, G::Navigate, None), i(Interaction::ScrollRight, G::Navigate, None),
                i(Interaction::Find, G::Search, Some("ctrl-f")), i(Interaction::FindSet, G::Search, Some("ctrl-shift-f")),
            ]);
            let o = scope.find.unwrap_or_default();
            out.extend([(Interaction::FindCase, "alt-c", o.case), (Interaction::FindWord, "alt-w", o.word), (Interaction::FindRegex, "alt-r", o.regex)]
                .map(|(command, key, on)| Entry { current: on, ..i(command, G::Search, Some(key)) }));
            if scope.surface == Surface::VersionPicker {
                out.extend(PickAct::ALL.iter().map(|&a| entry(Action::Pick(a), a.label(), G::Edit, None)));
            } else if !scope.image {
                out.extend(EditMenuCommand::ALL.iter().map(|&e| entry(Action::Edit(e), e.label(), G::Edit, match e {
                    EditMenuCommand::Undo => Some("ctrl-z"), EditMenuCommand::Redo => Some("ctrl-shift-z"), EditMenuCommand::Save => Some("ctrl-s"), _ => None })));
            }
            out.extend([Interaction::Copy, Interaction::CopyMarkdownSource, Interaction::CopyPath, Interaction::CopyReference,
                        Interaction::CopyLeftChange, Interaction::CopyRightChange, Interaction::SelectAll].map(|x| i(x, G::Copy, None)));
            out.extend([Interaction::Editor, Interaction::DefaultApp, Interaction::Reload].map(|x| i(x, G::File, None)));
            out.push(c(Command::LoadLarge, G::File, None));
            out.extend(FileAction::COMMANDS.iter().map(|&a| entry(Action::File(a), a.label(), G::File, None)));
            if scope.image {
                out.extend(crate::image_session::image_commands().iter().enumerate().map(|(n, e)| entry(Action::Image(n), e.label, G::View, None)));
            } else {
                if scope.markdown {
                    out.extend(markdown_views(scope).into_iter().map(|e| Entry { label: format!("View: {}", e.label).into(), ..e }));
                }
                let [w, h, s] = settings(scope);
                out.extend(w.into_iter().chain(h).chain(s));
                out.extend(diff_layout_menu(scope).into_iter().map(|e| Entry { label: format!("Diff layout: {}", e.label).into(), ..e }));
                out.extend([c(Command::ToggleContext, G::Diff, None), c(Command::ToggleCase, G::Diff, None)]);
                out.extend([i(Interaction::Blame, G::Diff, None), i(Interaction::InspectBlame, G::Diff, None)]);
                out.push(entry(Action::Help(HelpAction::DiffOptions), "Diff options…", G::Diff, None));
            }
            if scope.repository { out.extend([l(LayoutCommand::ToggleBranches, Some("alt-2")), l(LayoutCommand::ToggleHistory, Some("alt-3"))]); }
            out.extend([l(LayoutCommand::ToggleFiles, Some("alt-1")), i(Interaction::ResetPaneSizes, G::View, None)]);
            if scope.repository { out.push(if scope.focused { l(LayoutCommand::LeaveFocus, Some("escape")) } else { l(LayoutCommand::ToggleFocus, Some("f")) }); }
            if scope.repository {
                use crate::repository_session::{command_label, RepositoryCommand as R};
                out.extend([(RepoAction::Back, "Back", Some("alt-left")), (RepoAction::Forward, "Forward", Some("alt-right"))]
                    .map(|(a, label, key)| entry(Action::Repository(a), label, G::Navigate, key)));
                out.extend([(RepoAction::Refresh, command_label(&R::RefreshRepository)), (RepoAction::Branches, "Branches…"), (RepoAction::StagedChanges, command_label(&R::Index)),
                            (RepoAction::CopyUrl, command_label(&R::CopyUrl)), (RepoAction::OpenUrl, command_label(&R::OpenUrl)),
                            (RepoAction::Filter, "Filter history (messages and paths)…"), (RepoAction::LocalChanges, "Show local changes"),
                            (RepoAction::AllBranches, "History of all branches"), (RepoAction::CurrentBranch, "History of the current branch"),
                            (RepoAction::CompareBranch { merge_base: true }, "Compare selected branch with current (merge base)"),
                            (RepoAction::CompareBranch { merge_base: false }, "Compare selected branch with current (tips)"),
                            (RepoAction::CompareCommit, "Compare selected commit with selected branch"),
                            (RepoAction::NextParent, command_label(&R::Parent(0))), (RepoAction::FileHistory, command_label(&R::FileHistory(String::new()))),
                            (RepoAction::WriteGraph, command_label(&R::WriteGraph)), (RepoAction::CopyHash, command_label(&R::CopyHash)),
                            (RepoAction::CopySubject, command_label(&R::CopySubject)), (RepoAction::CopyDetails, command_label(&R::CopyDetails)),
                            (RepoAction::ShowInGraph, command_label(&R::ShowInGraph)), (RepoAction::OpenWorktree, "Open selected worktree"),
                            (RepoAction::ToggleMessage, command_label(&R::ToggleMessage(None)))]
                    .map(|(a, label)| entry(Action::Repository(a), label, G::Repository, None)));
            }
            if scope.pr {
                out.extend(DetailsTab::ALL.map(|t| entry(Action::Pr(PrAction::Details(t)), t.command_label(), G::PullRequest, None)));
                out.extend([(PrAction::MarkViewedNext, "Mark viewed & next file", Some("v")), (PrAction::Refresh, "Refresh pull request", Some("r")),
                            (PrAction::OutdatedThreads, "Outdated / file-level threads", Some("shift-t")), (PrAction::Threads, "Review threads of this file", Some("t")),
                            (PrAction::CheckHead, "Check PR head for updates", None)]
                    .map(|(a, label, key)| entry(Action::Pr(a), label, G::PullRequest, key)));
            }
        }
    }
    out.extend(app_menu());
    out
}

/// The diff header's switch: Source | Preview for Markdown, the comparison modes for images, else nothing.
pub fn header_switch(scope: &Scope) -> Vec<Entry> {
    if scope.surface != Surface::Comparison { return Vec::new(); }
    // a PR's reading tab hides the diff, so its modes wait
    let mut out: Vec<Entry> = Vec::new();
    if scope.pr && scope.focused && scope.pr_tab.is_some() { return out; }
    if scope.image {
        out.extend(crate::image_session::image_commands().iter().enumerate().filter_map(|(n, e)| match e.command {
            crate::image_session::ImageCommand::SetMode(mode) => Some(choice(Action::Image(n), mode.name().into(), Group::View, mode == scope.image_mode)),
            _ => None,
        }));
    } else if scope.markdown { out.extend(markdown_views(scope)); }
    out
}

/// A pull request's tabs in Focus, in GitHub's order: Conversation | Commits | Files changed (the diff). Both
/// toolbars show them in their middle.
pub fn pr_tabs(scope: &Scope) -> Vec<Entry> {
    if !(scope.pr && scope.focused) || scope.surface != Surface::Comparison { return Vec::new(); }
    let mut out: Vec<Entry> = DetailsTab::ALL.map(|t| choice(Action::Pr(PrAction::Details(t)), t.label().into(), Group::PullRequest, scope.pr_tab == Some(t))).into();
    out.push(choice(Action::Pr(PrAction::Diff), crate::pr_session::FILES_CHANGED.into(), Group::PullRequest, scope.pr_tab.is_none()));
    out
}

/// Source | Inline | Side by side: one row of modes, like the image modes.
fn markdown_views(scope: &Scope) -> Vec<Entry> {
    let source = Command::SetPresentation(PresentationMode::Source);
    let preview = scope.presentation == PresentationMode::Rendered;
    let mut out = vec![choice(Action::Comparison(source), source.label().into(), Group::View, !preview)];
    out.extend([MarkdownLayout::Inline, MarkdownLayout::SideBySide].map(|l| {
        choice(Action::Comparison(Command::Preview(l)), Command::Preview(l).label().into(), Group::View, preview && l == scope.markdown_layout)
    }));
    out
}

/// Split | Unified: the Diff menu's Layout row and the palette's entries.
pub fn diff_layout_menu(scope: &Scope) -> Vec<Entry> {
    [DiffLayout::Split, DiffLayout::Unified].map(|d| choice(Action::Comparison(Command::SetDiffLayout(d)), Command::SetDiffLayout(d).label().into(), Group::Diff, (d == DiffLayout::Unified) == scope.unified)).into()
}

/// Labels of the pointer menus' own rows (the change block, the Preview and blame under the pointer); both frontends.
pub mod pointer {
    use crate::view::Side;
    pub const SHOW_IN_SOURCE: &str = "Show in source";
    pub const HIDE_ANNOTATIONS: &str = "Hide annotations";
    pub const ANNOTATE_WITH_BLAME: &str = "Annotate with blame";
    pub fn fold(folded: bool) -> &'static str { if folded { "Unfold Change Block (x)" } else { "Fold Change Block (x)" } }
    /// A merge choice in the change block's menu; reopening reads as resetting it there.
    pub fn control(c: crate::merge_session::Control) -> &'static str {
        if matches!(c, crate::merge_session::Control::Reopen(_)) { "Reset this conflict resolution" } else { c.label() }
    }
    pub fn move_change(onto: Side) -> &'static str { if onto == Side::Right { "Move change → right (alt-shift-→)" } else { "Move change ← left (alt-shift-←)" } }
}

/// The ☰ menu: settings, theme, keys, about, error report. The palette lists them too.
pub fn app_menu() -> Vec<Entry> {
    let i = |command: Interaction| entry(Action::Interaction(command), command.label(), Group::Help, None);
    vec![
        entry(Action::Help(HelpAction::Settings), "Settings…", Group::Help, None),
        i(Interaction::ChangeTheme),
        entry(Action::Help(HelpAction::Shortcuts), "Keyboard shortcuts", Group::Help, Some("?")),
        i(Interaction::About),
        i(Interaction::ReportLastError),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn find_toggles_are_listed_with_their_state_and_need_an_open_find() {
        use lenz_content::find::FindOptions;
        let mut scope = Scope::comparison();
        scope.find = Some(FindOptions { case: true, ..Default::default() });
        let rows = entries(&scope);
        let case = rows.iter().find(|e| e.action == Action::Interaction(Interaction::FindCase)).unwrap();
        assert!(case.current && case.label == "Find: Match Case" && case.shortcut == Some("alt-c"));
        let word = rows.iter().find(|e| e.action == Action::Interaction(Interaction::FindWord)).unwrap();
        assert!(!word.current);
        let ctx = commands::InteractionContext { document: true, text: true, ..Default::default() };
        assert_eq!(Interaction::FindWord.disabled(&ctx), Some("No find is open"));
        assert_eq!(Interaction::FindWord.disabled(&commands::InteractionContext { find_open: true, ..ctx }), None);
    }

    use super::*;

    fn has(entries: &[Entry], action: Action) -> bool { entries.iter().any(|e| e.action == action) }

    #[test]
    fn focus_and_overview_entries_follow_the_mode() {
        let overview = Scope { repository: true, ..Scope::comparison() };
        assert!(has(&entries(&overview), Action::Layout(LayoutCommand::ToggleFocus)));
        assert!(!has(&entries(&overview), Action::Layout(LayoutCommand::LeaveFocus)));
        let focus = Scope { repository: true, focused: true, ..Scope::comparison() };
        assert!(has(&entries(&focus), Action::Layout(LayoutCommand::LeaveFocus)));
        assert!(!has(&entries(&focus), Action::Layout(LayoutCommand::ToggleFocus)));
        let plain = Scope::comparison();
        assert!(!has(&entries(&plain), Action::Layout(LayoutCommand::ToggleFocus)) && !has(&entries(&plain), Action::Layout(LayoutCommand::LeaveFocus)));
    }

    #[test]
    fn the_diff_menu_starts_with_the_layout() {
        let rows = diff_options(&Scope { unified: true, ..Scope::comparison() });
        assert_eq!(rows[0].label, "Layout");
        assert_eq!(rows[0].choices.iter().map(|c| (c.label.as_ref(), c.current)).collect::<Vec<_>>(), [("Split", false), ("Unified", true)]);
    }

    #[test]
    fn the_layout_choices_mark_the_current_one() {
        let scope = Scope::comparison();
        let layout: Vec<_> = diff_layout_menu(&scope).into_iter().map(|e| (e.label.into_owned(), e.current)).collect();
        assert_eq!(layout, [("Split".to_string(), true), ("Unified".to_string(), false)]);
        assert!(has(&entries(&scope), Action::Comparison(Command::SetDiffLayout(DiffLayout::Split))), "the palette lists the layouts");
    }

    #[test]
    fn diff_layout_menu_marks_the_current_layout() {
        let unified: Vec<_> = diff_layout_menu(&Scope { unified: true, ..Scope::comparison() }).into_iter().map(|e| e.current).collect();
        assert_eq!(unified, [false, true]);
        let view = crate::view::View::new(1, 1, &[], 10.0);
        assert_eq!(crate::commands::disabled(Command::SetDiffLayout(DiffLayout::Unified), &view, true, 0, 1, false), None, "the core allows it; adapters say when not");
    }

    #[test]
    fn repository_views_list_the_history_commands() {
        let e = entries(&Scope { repository: true, pr: true, ..Scope::comparison() });
        for a in [RepoAction::Filter, RepoAction::LocalChanges, RepoAction::AllBranches, RepoAction::CurrentBranch,
                  RepoAction::CompareBranch { merge_base: true }, RepoAction::CompareBranch { merge_base: false }, RepoAction::CompareCommit,
                  RepoAction::NextParent, RepoAction::FileHistory, RepoAction::WriteGraph, RepoAction::CopyHash, RepoAction::CopySubject,
                  RepoAction::CopyDetails, RepoAction::ShowInGraph, RepoAction::OpenWorktree] {
            assert!(has(&e, Action::Repository(a)), "{a:?}");
        }
        for a in [PrAction::Threads, PrAction::CheckHead] { assert!(has(&e, Action::Pr(a)), "{a:?}"); }
        assert!(!has(&entries(&Scope::comparison()), Action::Repository(RepoAction::AllBranches)));
    }

    #[test]
    fn comparison_lists_every_command_except_cycles() {
        let e = entries(&Scope { markdown: true, ..Scope::comparison() });
        for &c in commands::ALL {
            let cycle = matches!(c, Command::CycleScroll | Command::CycleWhitespace | Command::CycleHighlight);
            assert_eq!(has(&e, Action::Comparison(c)), !cycle, "{c:?}");
        }
        for &c in Interaction::ALL { assert!(has(&e, Action::Interaction(c)), "{c:?}"); }
        for &c in EditMenuCommand::ALL { assert!(has(&e, Action::Edit(c)), "{c:?}"); }
        for &c in FileAction::COMMANDS { assert!(has(&e, Action::File(c)), "{c:?}"); }
        for m in [WhitespaceMode::Default, WhitespaceMode::Trim, WhitespaceMode::Ignore, WhitespaceMode::IgnoreTrimChunks] {
            assert!(has(&e, Action::Comparison(Command::SetWhitespace(m))), "{m:?}");
        }
        for l in [HighlightLevel::Word, HighlightLevel::Char, HighlightLevel::Line] { assert!(has(&e, Action::Comparison(Command::SetHighlight(l)))); }
        for s in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded, ScrollMode::Independent] { assert!(has(&e, Action::Comparison(Command::SetScroll(s)))); }
        assert!(has(&entries(&Scope { repository: true, ..Scope::comparison() }), Action::Layout(LayoutCommand::ToggleFocus)));
        assert!(has(&e, Action::Help(HelpAction::Shortcuts)));
        for a in app_menu() { assert!(has(&e, a.action), "{:?} also in the palette", a.action); }
        assert_eq!(app_menu().iter().map(|e| e.label.as_ref()).collect::<Vec<_>>()[..2], ["Settings…", Interaction::ChangeTheme.label()]);
    }

    #[test]
    fn plain_comparison_has_no_repository_entries() {
        let e = entries(&Scope::comparison());
        assert!(!e.iter().any(|e| matches!(e.action, Action::Repository(_) | Action::Pr(_))));
        assert!(!has(&e, Action::Layout(LayoutCommand::ToggleBranches)));
        assert!(!has(&e, Action::Layout(LayoutCommand::ToggleHistory)));
        assert!(has(&e, Action::Layout(LayoutCommand::ToggleFiles)));
        assert!(!has(&e, Action::Comparison(Command::SetPresentation(PresentationMode::Rendered))), "not Markdown");
    }

    #[test]
    fn repository_and_pr_scopes_add_their_groups() {
        let e = entries(&Scope { repository: true, pr: true, ..Scope::comparison() });
        for a in [RepoAction::Refresh, RepoAction::Branches, RepoAction::StagedChanges, RepoAction::CopyUrl, RepoAction::OpenUrl, RepoAction::Back, RepoAction::Forward] { assert!(has(&e, Action::Repository(a))); }
        for t in DetailsTab::ALL { assert!(has(&e, Action::Pr(PrAction::Details(t)))); }
        for a in [PrAction::MarkViewedNext, PrAction::Refresh, PrAction::OutdatedThreads] { assert!(has(&e, Action::Pr(a))); }
        assert!(has(&e, Action::Layout(LayoutCommand::ToggleBranches)));
    }

    #[test]
    fn images_list_image_commands_instead_of_text_settings() {
        let e = entries(&Scope { image: true, ..Scope::comparison() });
        for i in 0..crate::image_session::image_commands().len() { assert!(has(&e, Action::Image(i)), "{i}"); }
        assert!(!e.iter().any(|e| matches!(e.action, Action::Comparison(Command::SetWhitespace(_)) | Action::Edit(_))));
        assert!(diff_options(&Scope { image: true, ..Scope::comparison() }).is_empty());
    }

    #[test]
    fn merge_and_version_picker_list_their_actions() {
        let e = entries(&Scope { surface: Surface::Merge, ..Scope::comparison() });
        for a in [Act::PrevChange, Act::NextChange, Act::PrevConflict, Act::NextConflict, Act::Find, Act::NextMatch, Act::PreviousMatch,
                  Act::NextPane, Act::ClearFind, Act::Undo, Act::Redo, Act::Save, Act::Done, Act::Cancel, Act::Reload, Act::ResolveSimple,
                  Act::ApplyNonConflicting, Act::AllFrom(Pick::Ours), Act::AllFrom(Pick::Theirs), Act::StartOver, Act::MarkResolved,
                  Act::ToggleBase, Act::Whitespace, Act::Scroll] { assert!(has(&e, Action::Merge(a)), "{a:?}"); }
        assert!(!e.iter().any(|e| matches!(e.action, Action::Comparison(_) | Action::Edit(_))));
        let e = entries(&Scope { surface: Surface::VersionPicker, markdown: true, ..Scope::comparison() });
        for a in PickAct::ALL { assert!(has(&e, Action::Pick(a)), "{a:?}"); }
        assert!(!e.iter().any(|e| matches!(e.action, Action::Edit(_))), "the picker's own actions replace Edit");
        for a in [Action::Comparison(Command::Preview(MarkdownLayout::Inline)), Action::Interaction(Interaction::Copy), Action::Comparison(Command::NextChange)] {
            assert!(has(&e, a), "{a:?} stays reachable in the Version Picker");
        }
    }

    #[test]
    fn current_values_are_marked_once_per_setting() {
        let mut scope = Scope::comparison();
        scope.options.whitespace = WhitespaceMode::Trim;
        let e = entries(&scope);
        let current: Vec<_> = e.iter().filter(|e| e.current && matches!(e.action, Action::Comparison(Command::SetWhitespace(_)))).collect();
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].action, Action::Comparison(Command::SetWhitespace(WhitespaceMode::Trim)));
        assert_eq!(current[0].label, "Whitespace: Trim");
    }

    #[test]
    fn the_preview_offers_inline_and_side_by_side() {
        use crate::markdown::MarkdownLayout;
        let md = |presentation, markdown_layout| Scope { markdown: true, presentation, markdown_layout, ..Scope::comparison() };
        let labels = |s: &Scope| header_switch(s).iter().map(|e| e.label.to_string()).collect::<Vec<_>>();
        // one row of modes, like the image modes
        let source = header_switch(&md(PresentationMode::Source, MarkdownLayout::Inline));
        assert_eq!(labels(&md(PresentationMode::Source, MarkdownLayout::Inline)), ["Source", "Inline", "Side by side"]);
        assert!(source[0].current && !source[1].current);
        let preview = header_switch(&md(PresentationMode::Rendered, MarkdownLayout::SideBySide));
        assert!(preview[2].current && !preview[0].current && !preview[1].current);
        assert_eq!(preview[1].action, Action::Comparison(Command::Preview(MarkdownLayout::Inline)));
        let palette = entries(&md(PresentationMode::Source, MarkdownLayout::Inline));
        for label in ["View: Source", "View: Inline", "View: Side by side"] {
            assert!(palette.iter().any(|e| e.label == label), "{label}");
        }
    }

    #[test]
    fn pr_tabs_list_githubs_order_in_focus_and_hide_the_diffs_modes() {
        let labels = |entries: Vec<Entry>| entries.into_iter().map(|e| (e.label.into_owned(), e.current)).collect::<Vec<_>>();
        let focus = Scope { repository: true, pr: true, focused: true, ..Scope::comparison() };
        assert_eq!(labels(pr_tabs(&focus)), [("Conversation".to_string(), false), ("Commits".to_string(), false), ("Files changed".to_string(), true)], "GitHub's order");
        let commits = Scope { pr_tab: Some(DetailsTab::Commits), ..focus };
        assert_eq!(labels(pr_tabs(&commits)).iter().filter(|(_, current)| *current).map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Commits"]);
        assert_eq!(pr_tabs(&focus)[2].action, Action::Pr(PrAction::Diff));
        assert!(pr_tabs(&Scope { focused: false, ..focus }).is_empty(), "Overview: no tabs");
        assert!(header_switch(&focus).is_empty(), "the tabs are not in the header");
        assert!(header_switch(&Scope { markdown: true, pr_tab: Some(DetailsTab::Conversation), ..focus }).is_empty(), "a reading tab hides the diff's own modes");
        assert_eq!(labels(header_switch(&Scope { markdown: true, ..focus }))[0].0, "Source", "Files changed keeps them");
    }

    #[test]
    fn header_switch_offers_markdown_views_or_image_modes() {
        assert!(header_switch(&Scope::comparison()).is_empty());
        let md = header_switch(&Scope { markdown: true, ..Scope::comparison() });
        assert_eq!(md.iter().map(|e| e.label.as_ref()).collect::<Vec<_>>(), ["Source", "Inline", "Side by side"]);
        assert!(md[0].current);
        let images = header_switch(&Scope { image: true, image_mode: crate::image_session::Mode::Swipe, ..Scope::comparison() });
        assert_eq!(images.iter().map(|e| e.label.as_ref()).collect::<Vec<_>>(), ["2-up", "Swipe", "Onion skin", "Difference"]);
        assert_eq!(images.iter().filter(|e| e.current).count(), 1);
        assert!(images[1].current);
        assert!(header_switch(&Scope { surface: Surface::Merge, markdown: true, ..Scope::comparison() }).is_empty());
    }

    #[test]
    fn diff_options_rows_match_the_popover() {
        let rows = diff_options(&Scope { blame: true, ..Scope::comparison() });
        let labels: Vec<_> = rows.iter().map(|r| r.label).collect();
        assert_eq!(labels, ["Layout", "Whitespace", "Highlight", "Scroll", "Show", "Compare"]);
        assert_eq!(rows[0].choices.len(), 2);
        assert_eq!(rows[1].choices.len(), 4);
        assert_eq!(rows[2].choices.len(), 3);
        assert_eq!(rows[3].choices.len(), 4);
        assert!(rows[5].choices.iter().any(|c| c.action == Action::Interaction(Interaction::Blame) && c.current));
    }
}
