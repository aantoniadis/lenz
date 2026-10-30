//! Semantic text-comparison commands. Both frontends use this decision point.
use lenz_engine::{HighlightLevel, Options, WhitespaceMode};
use crate::view::{View, ScrollMode, Side};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    PreviousChange, NextChange, PreviousFile, NextFile, ToggleContext, ToggleFold,
    UnfoldAll, CycleScroll, CycleWhitespace, CycleHighlight, ToggleCase, LoadLarge,
    SetPresentation(crate::markdown::PresentationMode),
    SetMarkdownLayout(crate::markdown::MarkdownLayout),
    /// The Markdown Preview in this layout: the header's Inline and Side by side.
    Preview(crate::markdown::MarkdownLayout),
    SetScroll(ScrollMode), SetWhitespace(WhitespaceMode), SetHighlight(HighlightLevel),
    SetDiffLayout(DiffLayout),
}

/// How a Comparison is laid out: two Sides next to each other, or one column (phase 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffLayout { #[default] Split, Unified }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect { Presentation(crate::markdown::PresentationMode), MarkdownLayout(crate::markdown::MarkdownLayout), Preview(crate::markdown::MarkdownLayout), Repaint, Recompare, Edge(bool), File(bool), LoadLarge }

pub const ALL: &[Command] = &[
    Command::SetPresentation(crate::markdown::PresentationMode::Source),
    Command::Preview(crate::markdown::MarkdownLayout::Inline),
    Command::Preview(crate::markdown::MarkdownLayout::SideBySide),
    Command::PreviousChange, Command::NextChange, Command::PreviousFile, Command::NextFile,
    Command::ToggleContext, Command::ToggleFold, Command::UnfoldAll, Command::CycleScroll,
    Command::CycleWhitespace, Command::CycleHighlight, Command::ToggleCase, Command::LoadLarge,
];

impl Command {
    pub fn label(self) -> &'static str {
        match self {
            Self::SetPresentation(crate::markdown::PresentationMode::Source) => "Source",
            Self::SetPresentation(crate::markdown::PresentationMode::Rendered) => "Preview",
            Self::SetMarkdownLayout(crate::markdown::MarkdownLayout::Inline) | Self::Preview(crate::markdown::MarkdownLayout::Inline) => "Inline",
            Self::SetMarkdownLayout(crate::markdown::MarkdownLayout::SideBySide) | Self::Preview(crate::markdown::MarkdownLayout::SideBySide) => "Side by side",
            Self::PreviousChange => "Previous change", Self::NextChange => "Next change",
            Self::PreviousFile => "Previous file", Self::NextFile => "Next file",
            Self::ToggleContext => "Toggle unchanged context", Self::ToggleFold => "Fold/unfold current change",
            Self::UnfoldAll => "Unfold all changes", Self::CycleScroll => "Cycle scroll mode",
            Self::CycleWhitespace => "Cycle whitespace mode", Self::CycleHighlight => "Cycle inline highlighting",
            Self::ToggleCase => "Toggle ignore case", Self::LoadLarge => "Load oversized comparison",
            Self::SetDiffLayout(DiffLayout::Split) => "Split", Self::SetDiffLayout(DiffLayout::Unified) => "Unified",
            Self::SetScroll(_) => "Set scroll mode", Self::SetWhitespace(_) => "Set whitespace mode", Self::SetHighlight(_) => "Set inline highlighting",
        }
    }
}

/// Why a comparison cannot show Unified now: images and the Markdown Preview have their own layouts. Both frontends.
pub fn unified_disabled(image: bool, rendered: bool) -> Option<&'static str> {
    if image { Some("Images have no Unified view") } else if rendered { Some("Switch the Preview back to Source first") } else { None }
}

/// Why commands that need a document are off in an empty window or a selection without changes.
pub const NOTHING_OPEN: &str = "No comparison open";

pub fn disabled(command: Command, view: &View, loading: bool, current: usize, count: usize, large: bool) -> Option<&'static str> {
    match command {
        Command::SetDiffLayout(_) => None,
        // Presentation switches are read-only even while a diff is pending. Their
        // document eligibility is validated by ComparisonSession::presentation_disabled.
        Command::SetPresentation(_) | Command::SetMarkdownLayout(_) | Command::Preview(_) => None,
        _ if count == 0 => Some("no comparison open"),
        Command::PreviousFile if current == 0 => Some("first file"),
        Command::NextFile if current + 1 >= count => Some("last file"),
        Command::PreviousFile | Command::NextFile => None,
        _ if loading => Some("comparison loading"),
        Command::ToggleFold if !view.current_change().is_some_and(|i| view.can_fold(i)) => Some("no multiline change selected"),
        Command::LoadLarge if !large => Some("no oversized comparison"),
        _ => None,
    }
}

pub fn execute(command: Command, view: &mut View, options: &mut Options, collapse: &mut bool) -> Effect {
    crate::diagnostics::breadcrumb(command.label());
    match command {
        Command::SetPresentation(mode) => return Effect::Presentation(mode),
        Command::SetMarkdownLayout(layout) => return Effect::MarkdownLayout(layout),
        Command::Preview(layout) => return Effect::Preview(layout),
        Command::SetDiffLayout(layout) => view.set_unified(layout == DiffLayout::Unified),
        Command::PreviousChange | Command::NextChange => {
            let forward = command == Command::NextChange;
            if !view.step_change(forward) { return Effect::Edge(forward); }
        }
        Command::PreviousFile | Command::NextFile => return Effect::File(command == Command::NextFile),
        Command::ToggleContext => { *collapse = !*collapse; view.set_collapsed(*collapse); }
        Command::ToggleFold => { if let Some(i) = view.current_change() { view.toggle_fold(i); } }
        Command::UnfoldAll => view.unfold_all(),
        Command::CycleScroll => view.set_mode(view.mode().next()),
        Command::SetScroll(mode) => view.set_mode(mode),
        Command::SetWhitespace(mode) => { if options.whitespace==mode{return Effect::Repaint;}options.whitespace = mode; return Effect::Recompare; }
        Command::SetHighlight(level) => { if options.highlight==level{return Effect::Repaint;}options.highlight = level; return Effect::Recompare; }
        Command::CycleWhitespace => {
            options.whitespace = match options.whitespace {
                WhitespaceMode::Default => WhitespaceMode::Trim, WhitespaceMode::Trim => WhitespaceMode::Ignore,
                WhitespaceMode::Ignore => WhitespaceMode::IgnoreTrimChunks, WhitespaceMode::IgnoreTrimChunks => WhitespaceMode::Default,
            };
            return Effect::Recompare;
        }
        Command::CycleHighlight => {
            options.highlight = match options.highlight {
                HighlightLevel::Word => HighlightLevel::Char, HighlightLevel::Char => HighlightLevel::Line, HighlightLevel::Line => HighlightLevel::Word,
            };
            return Effect::Recompare;
        }
        Command::ToggleCase => { options.ignore_case = !options.ignore_case; return Effect::Recompare; }
        Command::LoadLarge => return Effect::LoadLarge,
    }
    Effect::Repaint
}

pub fn whitespace_name(mode: WhitespaceMode) -> &'static str {
    match mode {
        WhitespaceMode::Default => "compare all",
        WhitespaceMode::Trim => "trim",
        WhitespaceMode::Ignore => "ignore",
        WhitespaceMode::IgnoreTrimChunks => "ignore + trim chunks",
    }
}

pub fn highlight_name(level: HighlightLevel) -> &'static str {
    match level {
        HighlightLevel::Word => "words",
        HighlightLevel::Char => "characters",
        HighlightLevel::Line => "lines only",
    }
}

pub fn scroll_name(mode: ScrollMode) -> &'static str {
    match mode {
        ScrollMode::Shared => "shared",
        ScrollMode::Aligned => "aligned",
        ScrollMode::Padded => "padded",
        ScrollMode::Independent => "independent",
    }
}

/// Interaction effects use the same labels and availability in both adapters.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum Interaction { Find, FindSet, FindCase, FindWord, FindRegex, GoToLine, Copy, CopyMarkdownSource, CopyPath, CopyReference, CopyLeftChange, CopyRightChange, SelectAll, Editor, DefaultApp, Blame, InspectBlame, Reload, ReportLastError, About, ChangeTheme, ScrollLeft, ScrollRight, ResetPaneSizes, StickyScroll }
#[derive(Default)] pub struct InteractionContext {pub document:bool,pub text:bool,pub selection:bool,pub repository:bool,pub annotation:bool,
    /// The comparison set has no files: [`NOTHING_OPEN`] rather than loading.
    pub empty:bool,
    /// A find or Change Set search input is open (the find option toggles apply to it).
    pub find_open:bool}
impl Interaction {
    pub const ALL:&'static [Self]=&[Self::Find,Self::FindSet,Self::FindCase,Self::FindWord,Self::FindRegex,Self::GoToLine,Self::Copy,Self::CopyMarkdownSource,Self::CopyPath,Self::CopyReference,Self::CopyLeftChange,Self::CopyRightChange,Self::SelectAll,Self::Editor,Self::DefaultApp,Self::Blame,Self::InspectBlame,Self::Reload,Self::ReportLastError,Self::About,Self::ChangeTheme,Self::ScrollLeft,Self::ScrollRight,Self::ResetPaneSizes,Self::StickyScroll];
    pub const fn label(self)->&'static str{match self{Self::ResetPaneSizes=>"Reset pane sizes",Self::StickyScroll=>"View: Sticky Scroll",Self::ScrollLeft=>"Scroll focused text pane left",Self::ScrollRight=>"Scroll focused text pane right",Self::Find=>"Find in focused side",Self::FindSet=>"Find in Change Set",Self::FindCase=>"Find: Match Case",Self::FindWord=>"Find: Whole Word",Self::FindRegex=>"Find: Regular Expression",Self::GoToLine=>"Go to line or line:column",Self::Copy=>"Copy selection",Self::CopyMarkdownSource=>"Copy Markdown source",Self::CopyPath=>"Copy file path",Self::CopyReference=>"Copy reference (path:line)",Self::CopyLeftChange=>"Copy left change",Self::CopyRightChange=>"Copy right change",Self::SelectAll=>"Select all on focused side",Self::Editor=>"Open focused side in editor",Self::DefaultApp=>"Open focused side with default app",Self::Blame=>"Toggle blame annotations",Self::InspectBlame=>"Inspect annotated commit",Self::ReportLastError=>"Report last error",Self::About=>crate::about::TITLE,Self::ChangeTheme=>"Change theme",Self::Reload=>"Reload comparison from disk"}}
    pub fn disabled(self,c:&InteractionContext)->Option<&'static str>{match self{Self::FindCase|Self::FindWord|Self::FindRegex if !c.find_open=>Some("No find is open"),Self::FindCase|Self::FindWord|Self::FindRegex=>None,Self::About|Self::ReportLastError|Self::ChangeTheme|Self::ResetPaneSizes|Self::StickyScroll=>None,Self::ScrollLeft|Self::ScrollRight|Self::Find|Self::GoToLine|Self::Copy|Self::CopyReference|Self::CopyLeftChange|Self::CopyRightChange|Self::SelectAll|Self::Blame|Self::InspectBlame if c.document&&!c.text=>Some("Not a text comparison"),Self::Copy if !c.selection=>Some("No selected text"),Self::Blame if !c.repository=>Some("Not a repository comparison"),Self::InspectBlame if !c.annotation=>Some("No annotation at selected line"),_ if !c.document&&c.empty=>Some(NOTHING_OPEN),_ if !c.document=>Some("Comparison loading"),_=>None}}
    /// The find option a toggle flips.
    pub const fn find_option(self)->Option<lenz_content::find::FindOption>{use lenz_content::find::FindOption as O;match self{Self::FindCase=>Some(O::Case),Self::FindWord=>Some(O::Word),Self::FindRegex=>Some(O::Regex),_=>None}}
}

/// Clipboard source text is computed once in the core; adapters only transport it.
pub struct CopyContext<'a>{pub set:&'a crate::changeset::ChangeSet,pub index:usize,pub doc:&'a crate::document::TextDocument,pub view:&'a View,pub selection:Option<crate::selection::Selection>,pub side:crate::view::Side,pub path_label:Option<&'a str>}
fn copy_change_range(command:Interaction,view:&View,selection:Option<crate::selection::Selection>,side:crate::view::Side)->Result<(crate::view::Side,std::ops::Range<u32>),&'static str>{
    let line=selection.filter(|s|s.side==side).map_or(view.anchor_line_on(side)as u32,|s|s.head.line);
    let block=view.block_index(side,line).or_else(||view.current_change()).ok_or("No change selected")?;
    let(left,right)=view.block_range(block).ok_or("No change selected")?;
    let result=if command==Interaction::CopyLeftChange{(crate::view::Side::Left,left)}else{(crate::view::Side::Right,right)};
    if result.1.is_empty(){Err("This change side is empty")}else{Ok(result)}
}
/// Availability must not materialize clipboard text just to paint a menu.
pub fn copy_disabled(command:Interaction,context:CopyContext<'_>)->Option<&'static str>{
    if context.set.comparisons.get(context.index).is_none(){return Some("No file selected");}
    match command{
        Interaction::CopyMarkdownSource=>Some("Select text in Rendered mode"),
        Interaction::Copy if context.selection.is_none_or(|s|s.is_empty())=>Some("No selected text"),
        Interaction::Copy|Interaction::CopyPath|Interaction::CopyReference=>None,
        Interaction::CopyLeftChange|Interaction::CopyRightChange=>copy_change_range(command,context.view,context.selection,context.side).err(),
        _=>Some("Not a clipboard command")
    }
}
pub fn copy_text(command:Interaction,context:CopyContext<'_>)->Result<String,&'static str>{
    let CopyContext{set,index,doc,view,selection,side,path_label}=context;
    let c=set.comparisons.get(index).ok_or("No file selected")?;let path=path_label.unwrap_or(if side==Side::Left{&c.old_path}else{&c.path});
    let line=selection.filter(|s|s.side==side).map_or(view.anchor_line_on(side)as u32,|s|s.head.line);
    match command{
        Interaction::Copy=>selection.filter(|s|!s.is_empty()).map(|s|s.text(doc.lines_on(s.side))).ok_or("No selected text"),
        Interaction::CopyPath=>Ok(path.into()),Interaction::CopyReference=>Ok(format!("{path}:{}",line+1)),
        Interaction::CopyLeftChange|Interaction::CopyRightChange=>{
            let(side,range)=copy_change_range(command,view,selection,side)?;
            Ok(doc.lines_on(side)[range.start as usize..range.end as usize].join("\n"))
        },_=>Err("Not a clipboard command")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn reset_pane_sizes_is_available_without_a_document() {
        let command = super::Interaction::ResetPaneSizes;
        assert!(super::Interaction::ALL.contains(&command));
        assert_eq!(command.label(), "Reset pane sizes");
        assert_eq!(command.disabled(&super::InteractionContext::default()), None);
    }

    use super::*;
    use crate::document::TextDocument;
    use std::path::Path;

    #[test]
    fn presentation_commands_are_discoverable_with_shared_labels() {
        use crate::markdown::MarkdownLayout as L;
        for (command,label) in [(Command::SetPresentation(crate::markdown::PresentationMode::Source),"Source"),(Command::Preview(L::Inline),"Inline"),(Command::Preview(L::SideBySide),"Side by side")] {
            assert_eq!(ALL.iter().filter(|&&entry|entry==command).count(),1);assert_eq!(command.label(),label);
        }
    }

    #[test]
    fn commands_fold_only_multiline_changes_and_unfold_again() {
        let doc = TextDocument::new("a\nb\n", "x\ny\n", Path::new("a"), Path::new("b"));
        let mut options = Options::default();
        let mut view = doc.view(&doc.line_diff(&options), 20.0);
        let mut collapse = false;
        view.jump_to_first_change();
        execute(Command::ToggleFold, &mut view, &mut options, &mut collapse);
        assert!(view.folded(0));
        execute(Command::UnfoldAll, &mut view, &mut options, &mut collapse);
        assert!(!view.folded(0));
        let doc = TextDocument::new("a\n", "x\n", Path::new("a"), Path::new("b"));
        let mut view = doc.view(&doc.line_diff(&options), 20.0);
        view.jump_to_first_change();
        execute(Command::ToggleFold, &mut view, &mut options, &mut collapse);
        assert!(!view.folded(0));
    }

    #[test]
    fn changing_comparison_options_requests_a_new_diff() {
        let mut view = View::new(0, 0, &[], 20.0);
        let mut options = Options::default();
        let mut collapse = false;
        assert_eq!(execute(Command::ToggleCase, &mut view, &mut options, &mut collapse), Effect::Recompare);
        assert!(options.ignore_case);
        execute(Command::CycleWhitespace, &mut view, &mut options, &mut collapse);
        assert_eq!(options.whitespace, WhitespaceMode::Trim);
    }
}
