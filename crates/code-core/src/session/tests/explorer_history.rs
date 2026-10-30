use super::*;
use crate::workspace::Explorer;
use lenz_content::git::testing::{commit_all, git, repo, write as put};

fn highlight(s: &mut Session, path: Option<&str>) {
    s.dispatch(Command::SetExplorer(Explorer { visible: true, selected: path.map(str::to_string), top: None }));
}

#[test]
fn explorer_history_targets_the_selected_file_or_folder_in_the_enclosing_repository() {
    let r = repo();
    put(r.path(), "project space/dir[one]/-selected?.txt", "selected\n");
    commit_all(r.path(), "Selected file");
    let first = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
    put(r.path(), "project space/dirone/-selectedX.txt", "decoy\n");
    put(r.path(), "outside.txt", "outside\n");
    commit_all(r.path(), "Outside the selected directory");
    let bin = tempfile::tempdir().unwrap();
    for name in ["lenz-diff", "lenz-diff-tui"] { std::fs::write(bin.path().join(name), "").unwrap(); }
    let exe = bin.path().join("lenz-code");
    let mut s = session(bin.path());
    s.dispatch(Command::OpenRoot(r.path().join("project space")));
    open(&mut s, Command::Open(r.path().join("outside.txt")), "outside.txt");
    settle(&mut s, |s| s.state.repo.is_some() && s.state.project.as_ref().is_some_and(|p| p.has_file("dir[one]/-selected?.txt")));
    highlight(&mut s, Some("dir[one]/-selected?.txt"));
    assert_eq!(s.explorer_history_disabled(), None);
    let launch = s.explorer_history_launch(&exe, false).unwrap();
    assert_eq!(launch.args, ["log", "--follow", "project space/dir[one]/-selected?.txt"]);
    assert_eq!(launch.cwd.as_deref(), Some(r.path().canonicalize().unwrap().as_path()));
    assert_eq!(git(launch.cwd.as_deref().unwrap(), &["--literal-pathspecs", "log", "--format=%H", "--follow", "--", &launch.args[2]]).trim(), first);
    let tui = s.explorer_history_launch(&exe, true).unwrap();
    assert_eq!(tui.args, launch.args);
    assert_eq!(tui.cwd, launch.cwd);
    assert!(tui.program.ends_with("lenz-diff-tui"));

    highlight(&mut s, Some("dir[one]"));
    let launch = s.explorer_history_launch(&exe, false).unwrap();
    assert_eq!(launch.args, ["log", "--", ":(literal)project space/dir[one]"]);
    assert_eq!(git(launch.cwd.as_deref().unwrap(), &["log", "--format=%H", "--", &launch.args[2]]).trim(), first);
    for selected in [None, Some("")] {
        highlight(&mut s, selected);
        assert_eq!(s.explorer_history_launch(&exe, false).unwrap().args, ["log", "--", ":(literal)project space"]);
    }
    assert_eq!(s.state.active().unwrap().source(), r.path().join("outside.txt"));
}

#[test]
fn explorer_history_rejects_foreign_stale_and_non_repository_entries() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let path = write(&root, "a.txt", b"a\n");
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root));
    open(&mut s, Command::Open(path), "a.txt");
    settle(&mut s, |s| s.state.repo_answered == s.state.repo_anchor);
    assert_eq!(s.explorer_history_disabled(), Some("Not in a Git repository"));
    assert!(s.explorer_history_launch(&d.path().join("lenz-code"), false).unwrap_err().contains("Not in a Git repository"));
    for selected in ["../outside", "/outside", "language://virtual"] {
        highlight(&mut s, Some(selected));
        assert_eq!(s.explorer_history_disabled(), Some("Highlight a file or folder in the open project"));
    }
    highlight(&mut s, Some("gone.txt"));
    assert_eq!(s.explorer_history_disabled(), Some("The highlighted file or folder is no longer in the project"));
}

#[test]
fn explorer_history_root_pathspec_and_leading_magic_names_are_literal() {
    let r = repo();
    put(r.path(), ":(glob)*.txt", "literal name\n");
    commit_all(r.path(), "Literal name");
    let expected = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
    put(r.path(), "other.txt", "other\n");
    commit_all(r.path(), "Other file");
    let bin = tempfile::tempdir().unwrap();
    std::fs::write(bin.path().join("lenz-diff"), "").unwrap();
    let mut s = session(bin.path());
    s.dispatch(Command::OpenRoot(r.path().to_path_buf()));
    settle(&mut s, |s| s.state.repo.is_some() && s.state.project.as_ref().is_some_and(|p| p.has_file(":(glob)*.txt")));
    highlight(&mut s, Some(":(glob)*.txt"));
    let launch = s.explorer_history_launch(&bin.path().join("lenz-code"), false).unwrap();
    assert_eq!(git(launch.cwd.as_deref().unwrap(), &["--literal-pathspecs", "log", "--format=%H", "--follow", "--", &launch.args[2]]).trim(), expected);
    highlight(&mut s, None);
    let launch = s.explorer_history_launch(&bin.path().join("lenz-code"), false).unwrap();
    assert_eq!(launch.args, ["log", "--", ":(literal)."]);
    assert_eq!(git(launch.cwd.as_deref().unwrap(), &["log", "--format=%H", "--", &launch.args[2]]).lines().count(), 2);
    std::fs::remove_file(bin.path().join("lenz-diff")).unwrap();
    assert!(s.explorer_history_launch(&bin.path().join("lenz-code"), false).unwrap_err().contains("reinstall Lenz"));
}
