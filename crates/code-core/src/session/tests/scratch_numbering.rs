use super::*;

#[test]
fn exhausted_scratch_numbering_reports_an_error_without_writing_or_switching_files() {
    let state = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let active = write(project.path(), "active.txt", b"keep the active source\r\n");
    let mut s = session(state.path());
    open(&mut s, Command::OpenRetained(active.clone()), "active.txt");
    let tab = s.state().active().unwrap().id();
    let before = s.state().current_source().unwrap().key();
    assert!(s.scratch_files().unwrap().is_empty());
    let maximum = s.scratch_directory().join(format!("scratch_{}.rb", u64::MAX));
    let ordinary = s.scratch_directory().join("scratch_1.json");
    std::fs::write(&maximum, b"puts 'preserve maximum'\n").unwrap();
    std::fs::write(&ordinary, b"{\"preserve\":true}\r\n").unwrap();
    let files_before = s.scratch_files().unwrap();
    let ruby = crate::scratch::KINDS.iter().position(|kind| kind.label == "Ruby").unwrap();

    let effect = s.dispatch(Command::NewScratchFile(ruby));

    assert!(effect.repaint);
    assert!(s.status_details().contains("Scratch file numbering is exhausted"), "{}", s.status_details());
    assert!(s.status_details().contains("Existing files were not changed"));
    assert_eq!(s.state().active().unwrap().id(), tab);
    assert_eq!(s.state().current_source().unwrap().key(), before);
    assert_eq!(s.scratch_files().unwrap(), files_before, "no wrapped scratch_0 file is created");
    assert_eq!(std::fs::read(maximum).unwrap(), b"puts 'preserve maximum'\n");
    assert_eq!(std::fs::read(ordinary).unwrap(), b"{\"preserve\":true}\r\n");
    assert_eq!(std::fs::read(active).unwrap(), b"keep the active source\r\n");
}
