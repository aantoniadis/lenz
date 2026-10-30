//! Words shared by the desktop and terminal's input and confirmation dialogs.
use crate::{create, relocate, session::{PasteEntry, Session}, trash};

pub enum Form<'a> {
    RenameTerminal,
    Create(create::Kind, &'a str),
    SaveAs,
    Scratch,
    Relocate(relocate::Kind, &'a str),
    Paste(&'a PasteEntry),
    Delete(trash::Removal, &'a str),
    Branch(Option<&'a str>),
    Revision,
    GoTo,
    Workspace,
    SqlMetadata,
}

pub struct FormText {
    pub title: &'static str,
    pub field: Option<&'static str>,
    pub description: String,
    pub submit: &'static str,
    pub destructive: bool,
}

impl Session {
    pub fn form_text(&self, form: Form<'_>) -> FormText {
        let folder = |dir: &str| if dir.is_empty() { "the project root" } else { dir }.to_string();
        let (title, field, description, submit, destructive) = match form {
            Form::RenameTerminal => ("Rename Terminal Tab", Some("Name"), "Change this tab's label. Its shell and history stay running.".into(), "Rename", false),
            Form::Create(kind, dir) => (kind.label(), Some("Name"), format!("Create in {}", folder(dir)), "Create", false),
            Form::SaveAs => ("Save As", Some("File path"), "Save the current draft to a new location.".into(), "Save", false),
            Form::Scratch => ("New Scratch File", Some("Name"), "A personal file outside your project. Include an extension to choose its language.".into(), "Create", false),
            Form::Relocate(kind, from) => (kind.label(), Some(match kind { relocate::Kind::Rename => "Name", relocate::Kind::Move => "Destination folder" }), format!("{from}\n{}References in other files will not be changed.", if kind == relocate::Kind::Move { "Use a project-relative folder; leave empty for the root.\n" } else { "" }), kind.label(), false),
            Form::Paste(request) => ("Paste File / Folder", Some("Name"), format!("Copy {} into {}.\nCopies saved contents.", request.source().display(), folder(request.folder())), "Copy", false),
            Form::Delete(removal, from) => (removal.label(), None, self.delete_prompt(removal, from), removal.label(), true),
            Form::Branch(from) => ("New Branch", Some("Branch name"), format!("Create from {}", from.map_or("HEAD", crate::branches::short_ref)), "Create", false),
            Form::Revision => ("Checkout Revision", Some("Revision"), "Enter a tag, branch, commit ID or expression such as HEAD~2.".into(), "Check Out", false),
            Form::GoTo => ("Go to Line", Some("Line : column"), "Enter a line number, optionally followed by :column.".into(), "Go", false),
            Form::Workspace => ("Language Server Workspace", Some("Folder path"), "Leave empty to restore automatic workspace discovery.".into(), "Apply", false),
            Form::SqlMetadata => ("Configure SQL Metadata", Some("Source"), self.sql_metadata_prompt().into(), "Configure", false),
        };
        FormText { title, field, description, submit, destructive }
    }
}
