use super::*;

#[test]
fn server_install_requires_explicit_start_and_is_fenced_to_previewed_provider() {
    let d = tempfile::tempdir().unwrap();
    let bash = write(d.path(), "script.sh", b"echo hello\n");
    let text = write(d.path(), "notes.txt", b"unchanged\n");
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(bash.clone()), "script.sh");
    s.dispatch(Command::OpenServerInstall);
    let install = s.state().server_install().unwrap();
    assert_eq!(install.provider, "bash-language-server");
    assert!(!install.running);
    assert!(install.text().contains("Start Installation"));
    assert!(install.preview.command.contains("bash-language-server@5.8.1"));
    open(&mut s, Command::OpenRetained(text.clone()), "notes.txt");
    s.dispatch(Command::StartServerInstall);
    assert!(!s.state().server_install().unwrap().running);
    assert!(s.state().server_install().unwrap().message.contains("changed"));
    s.dispatch(Command::CloseServerInstall);
    assert!(s.state().server_install().is_none());
    assert_eq!(std::fs::read(bash).unwrap(), b"echo hello\n");
    assert_eq!(std::fs::read(text).unwrap(), b"unchanged\n");
}

#[test]
fn server_install_close_keeps_cancellation_ownership_and_rejects_stale_progress() {
    let d=tempfile::tempdir().unwrap();let file=write(d.path(),"script.sh",b"echo unchanged\n");let mut s=session(d.path());
    open(&mut s,Command::OpenRetained(file),"script.sh");s.dispatch(Command::OpenServerInstall);
    let request=s.state.server_install.as_ref().unwrap().request;
    let cancel=s.state.server_install.as_ref().unwrap().cancel.clone();
    // Model the owned worker after Start, without any network package manager.
    s.state.server_install.as_mut().unwrap().running=true;s.install_running=Some((request,"bash-language-server",cancel.clone()));
    s.dispatch(Command::CloseServerInstall);
    assert!(cancel.load(Ordering::Relaxed));
    assert!(s.disabled(CommandId::InstallServer).is_some(),"a replacement install must wait for the old process to stop");
    s.dispatch(Completion::ServerInstallOutput {request,text:"obsolete".into()});
    assert!(s.state.server_install.is_none());
    s.dispatch(Completion::ServerInstalled {request,result:Err("cancelled".into())});
    assert!(s.install_running.is_none());
    s.dispatch(Command::OpenServerInstall);
    s.dispatch(Completion::ServerInstallOutput {request,text:"obsolete".into()});
    assert!(s.state.server_install.as_ref().unwrap().output.is_empty());
}
