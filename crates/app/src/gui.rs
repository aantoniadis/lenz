use crate::{changeset, cli, open, pr, ui, view};

use std::{process::ExitCode, time::Instant};

use gpui::{BorrowAppContext, App, AppContext, TitlebarOptions, WindowOptions};

pub fn run() -> ExitCode {
    crate::diagnostics::install("GUI");
    let (settings, notices) = crate::settings::load();
    crate::settings::init(settings);
    crate::settings::stash_notices(notices);
    let started = Instant::now();
    let timings = std::env::var_os("LENZ_TIMINGS").is_some();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = crate::diagnostics::report_cli(&args) { return code; }
    if cli::wants_help(&args) { println!("{}", cli::USAGE); return ExitCode::SUCCESS; }
    if let Some(code) = choose_repository(&args) { return code; }
    let (mut request, mut set, launch) = match load(&args) {
        Ok(loaded) => loaded,
        Err(LoadError::Usage(msg)) => {
            crate::diagnostics::user_error(&msg, "Check the command arguments; lenz --help lists them.");
            return ExitCode::from(2);
        }
        Err(LoadError::Failed(msg)) => {
            crate::diagnostics::failure("Open comparison", &msg, "Check the command arguments, input permissions and Git revisions, then retry.");
            return ExitCode::from(2);
        }
    };
    // the Change Set Picker, when a repository stands behind the Change Set: (root, shown)
    let mut picker: Option<(std::path::PathBuf, bool)> = None;
    let log_branches=matches!(request.launch,cli::Launch::Log{..}).then(||crate::repository_session::initial_log_branches(&request.spec));
    let (mut opened_pr, mut no_pr) = (None, false);
    let (mut merging, mut picking) = (None, None);
    // the Conflict List to start on, and the paths it's limited to
    let mut resolving = None;
    let root = set.git_root().map(std::path::Path::to_path_buf);
    match (&request.launch, root) {
        (cli::Launch::Merge(cli::Merge::Files { base, ours, theirs, merged }), _) => {
            let cwd = std::env::current_dir().unwrap_or_default();
            match ui::version_picker::open_files(base, ours, theirs, merged, &cwd) {
                Ok(Some(o)) => picking = Some(o),
                Ok(None) => match ui::merge_view::open_files(base, ours, theirs, merged, &cwd) {
                    Ok(o) => merging = Some(o),
                    Err(e) => {
                        crate::diagnostics::failure("Open merge", &e, "Check that all merge inputs are readable and the output directory is writable, then retry.");
                        return ExitCode::from(2);
                    }
                },
                Err(e) => {
                    crate::diagnostics::failure("Open merge", &e, "Check that all merge inputs are readable and the output directory is writable, then retry.");
                    return ExitCode::from(2);
                }
            }
            // the 2-way Comparison underneath is never shown; it keeps the viewer's machinery fed
            let spec = changeset::Spec::Plain { left: ours.clone(), right: theirs.clone() };
            set = match changeset::ChangeSet::load(&spec, &cwd) {
                Ok(s) if !s.comparisons.is_empty() => s,
                _ => changeset::ChangeSet::load(&changeset::Spec::Plain { left: base.clone(), right: merged.clone() }, &cwd).unwrap_or(set),
            };
            // the Version Picker's `b`: the Base on the left of the view below it
            if picking.is_some() {
                let spec = changeset::Spec::Plain { left: base.clone(), right: theirs.clone() };
                if let Ok(b) = changeset::ChangeSet::load(&spec, &cwd) {
                    set.comparisons.extend(b.comparisons);
                }
            }
        }
        (cli::Launch::Merge(cli::Merge::List(paths)), root) => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let Some(root) = root.or_else(|| changeset::toplevel(&cwd).ok()) else {
                eprintln!("lenz merge: not inside a git repository");
                return ExitCode::from(2);
            };
            let filter = ui::conflict_list::from_top(&root, &cwd, paths);
            match conflict_list(&root, &filter) {
                Ok(Some((s, snap))) => {
                    set = s;
                    picker = Some((root, true));
                    resolving = Some((snap, filter));
                }
                Ok(None) => {
                    eprintln!("No conflicts in {}", root.display());
                    return ExitCode::from(2);
                }
                Err(e) => {
                    crate::diagnostics::failure("Load conflicts", &e, "Check Git status and repository permissions, then retry.");
                    return ExitCode::from(2);
                }
            }
        }
        (cli::Launch::Pr(target), root) => {
            let root = root.or_else(|| changeset::toplevel(&std::env::current_dir().unwrap_or_default()).ok());
            let Some(root) = root else {
                eprintln!("lenz pr: run it inside a clone of the pull request's repository");
                return ExitCode::from(2);
            };
            match ui::pr_view::open(&root, target.as_deref()) {
                Ok(o) => {
                    set = o.set;
                    opened_pr = Some((o.pr, o.fetched));
                    picker = Some((root, true));
                }
                Err(pr::PrError::NoPullRequest) => {
                    // no PR for this branch: the picker, whose sidebar lists the open ones
                    if let Ok((initial,spec,_))=crate::repository_session::pinned_commit_comparison(&root,"HEAD"){
                        set=initial;request.spec=spec;
                    }
                    picker = Some((root, true));
                    no_pr = true;
                }
                Err(e) => {
                    crate::diagnostics::pull_request_failure(&e);
                    return ExitCode::from(2);
                }
            }
        }
        (cli::Launch::Log { .. }, Some(root)) => {
            if let Ok((initial,spec,_))=crate::repository_session::initial_log_comparison(&root,&request.spec){
                set=initial;request.spec=spec;
            }
            picker=Some((root,true));
        }
        (cli::Launch::Diff, Some(root)) => {
            // `lenz` in a clean repository browses history instead of saying "no differences"
            let bare = matches!(&request.spec, changeset::Spec::Git { cached: false, revs, paths, .. } if revs.is_empty() && paths.is_empty());
            // during a merge, rebase, cherry-pick, revert or stash pop: its Conflict List
            let conflicts = if bare { conflict_list(&root, &[]).ok().flatten() } else { None };
            if let Some((s, snap)) = conflicts {
                set = s;
                picker = Some((root, true));
                resolving = Some((snap, Vec::new()));
            } else if set.comparisons.is_empty() && bare {
                if let Ok((initial,spec,_))=crate::repository_session::initial_repository_comparison(&root){
                    set=initial;request.spec=spec;
                }
                picker = Some((root, true));
            } else {
                picker = Some((root, false));
            }
        }
        _ => {}
    }
    if set.comparisons.is_empty() && request.launch!=cli::Launch::About {
        if merging.is_some() || picking.is_some() {
            eprintln!("lenz merge: nothing to merge, the versions are identical");
            return ExitCode::from(2);
        }
        eprintln!("no differences");
        return ExitCode::SUCCESS;
    }
    // the first Comparison loads and diffs before the window opens; others load in the background
    let first = set.comparisons.first();
    let label = |p: &str| std::path::PathBuf::from(request.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| p.to_string()));
    let initial_doc = if let Some(first)=first { if merging.is_some() {
        // The Merge View owns all three versions. Its hidden 2-way view needs no second copy or diff.
        Ok(ui::Doc::new("", "", &label(&first.old_path), &label(&first.path)))
    } else {
        ui::Doc::load(&set, 0, &label(&first.old_path), &label(&first.path), false)
    }
    } else {Ok(ui::Doc::new("","",std::path::Path::new(""),std::path::Path::new("")))};
    let doc = match initial_doc {
        Ok(doc) => std::sync::Arc::new(doc),
        Err(e) => {
            crate::diagnostics::failure("Read comparison", &e, "Check that the input files are readable, then retry.");
            return ExitCode::from(2);
        }
    };
    let loaded = started.elapsed();
    // a pathological pair gives up after the same 2 s as later files, showing the text alone
    let (tx, rx) = std::sync::mpsc::channel();
    let (diffing, options) = (doc.clone(), request.options);
    std::thread::spawn(move || tx.send(diffing.line_diff(&options)));
    let blocks = rx.recv_timeout(std::time::Duration::from_secs(2)).ok();
    let mut view = doc.view(blocks.as_deref().unwrap_or_default(), 40.0);
    view.start_in(request.scroll);
    if blocks.is_some() {
        view.set_collapsed(true);
        view.jump_to_first_change();
    }
    if let Some(line) = request.line {
        view.go_to_line(view::Side::Right, line);
    }
    if blocks.is_some() {
        doc.refine_visible(&mut view, &request.options, &mut lenz_engine::Budget::default());
    }
    if timings {
        eprintln!(
            "lenz: load {:.0} ms, diff {:.0} ms",
            loaded.as_secs_f64() * 1000.0,
            (started.elapsed() - loaded).as_secs_f64() * 1000.0
        );
    }

    // stale "open in another app" copies go at startup, never on exit (an app may still have one open)
    std::thread::spawn(|| {
        if let Some(dir) = open::cache_dir(&open::Host::current()) {
            open::sweep(&dir, std::time::SystemTime::now());
        }
    });
    let start_rendered = request.markdown == crate::markdown::PresentationMode::Rendered && merging.is_none() && picking.is_none();
    gpui_platform::application().run(move |cx: &mut App| {
        let (font, font_notice) = ui::pick_mono(cx);
        if let Some(notice) = font_notice { crate::settings::stash_notices(vec![notice]); }
        let options = WindowOptions {
            app_id:Some("lenz".into()),
            focus: true,
            titlebar: Some(TitlebarOptions { title: Some("Lenz".into()), ..Default::default() }),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            dock_icon_after_first_frame(window);
            ui::themes::install(cx);
            let dark=matches!(window.appearance(),gpui::WindowAppearance::Dark|gpui::WindowAppearance::VibrantDark);
            cx.update_global::<ui::themes::GuiThemes,_>(|g,_|{g.0.execute(crate::theme::ThemeCommand::NativeAppearance(dark));});
            let set = std::sync::Arc::new(set);
            // the shared repository session, opened on the comparison loaded above; a followed file is
            // opened by the session once the sidebar inventory has resolved the repository
            let follow = match &request.launch { cli::Launch::Log { follow } => follow.clone(), _ => None };
            let session = picker.map(|(root, shown)| crate::repository_session::RepositorySession::open_loaded(root, shown, log_branches, follow, set.clone(), request.spec.clone()));
            let viewer = cx.new(|cx| {
                let mut v = ui::Viewer::new(set, request.spec, request.blame, request.name, doc, view, blocks.is_none(), request.options, font, started, timings, cx);
                if let Some(session) = session {
                    v.with_session(session, cx);
                    if let Some((found, fetched)) = opened_pr {
                        v.adopt_pr(found, fetched, cx);
                    }
                    if no_pr {
                        v.notice("No pull request for this branch — pick one under Pull requests".into(), cx);
                    }
                }
                v.restore_place(launch, cx);
                if let Some(o) = merging {
                    v.start_merge(o, true, cx);
                }
                if let Some(o) = picking {
                    v.start_picker(o, true, cx);
                }
                if let Some((snap, filter)) = resolving {
                    v.start_conflicts(snap, filter, cx);
                }
                if request.launch==cli::Launch::About{v.interaction(crate::commands::Interaction::About,cx);}
                if start_rendered { v.start_rendered(cx); }
                if let Some(notice) = crate::settings::take_notice() { v.notice(notice, cx); }
                v.prepare_small_syntax(cx);
                v
            });
            // the title bar's close button asks first when a merge has unsaved (or, under mergetool, un-Done) work
            let closing = viewer.clone();
            window.on_window_should_close(cx, move |_, cx| {
                closing.update(cx, |v, cx| v.edits_may_close(cx) && v.merge_may_close(cx) && v.picker_v_may_close(cx) && v.conflicts_may_close(cx))
            });
            // the palette follows the system's light/dark setting live
            viewer.update(cx, |_, cx| cx.observe_window_appearance(window, |_, window, cx| {
                let dark=matches!(window.appearance(), gpui::WindowAppearance::Dark|gpui::WindowAppearance::VibrantDark);
                cx.update_global::<ui::themes::GuiThemes,_>(|g,_|{g.0.execute(crate::theme::ThemeCommand::NativeAppearance(dark));});
            }).detach());
            // coming back to the window: has the pull request moved on, has the terminal resolved or continued, has the
            // merged file changed on disk?
            viewer.update(cx, |_, cx| {
                cx.observe_window_activation(window, |v, window, cx| {
                    if window.is_window_active() {
                        v.refresh_title(cx);
                        v.check_pr_moved(cx);
                        v.refresh_conflicts(cx);
                        v.check_merge_on_disk(cx);
                    }
                })
                .detach()
            });
            window.set_window_title(&viewer.read(cx).title());
            let focus = viewer.read(cx).focus_handle();
            window.focus(&focus, cx);
            viewer
        })
        .expect("open window");
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        finish_on_terminate(cx, ui::merge_view::exit_code);
        cx.activate(true);
    });
    ExitCode::from(ui::merge_view::exit_code())
}

/// What a launch does once its event loop ends, run where the platform lets it.
///
/// On macOS, quitting goes through `[NSApp terminate:]`, which ends the process with `exit(0)` after
/// gpui's quit observers: `run` never returns, so code after it never ran there. A cancelled
/// `git mergetool` merge exited 0 and git staged its conflict markers, and a repository chosen in
/// the chooser was never opened. This observer's future runs after gpui has torn down the windows,
/// where AppKit would exit next, and runs `finish` and exits with its code instead. Elsewhere `run`
/// returns and the caller runs `finish` itself.
fn finish_on_terminate(cx: &mut App, finish: impl Fn() -> u8 + 'static) {
    #[cfg(target_os = "macos")]
    cx.on_app_quit(move |_| {
        let code = finish();
        async move {
            use std::io::Write as _;
            let _ = std::io::stdout().flush();
            std::process::exit(code.into())
        }
    })
    .detach();
    #[cfg(not(target_os = "macos"))]
    let _ = (cx, finish);
}

/// Outside a repository, a launch that needs one opens the repository chooser; once one is chosen, Lenz starts again
/// inside it with the same arguments. `None` when no choice is needed.
fn choose_repository(args: &[String]) -> Option<ExitCode> {
    let cwd = std::env::current_dir().ok()?;
    let request = cli::parse(args, &cwd).ok()?;
    if !crate::repo_picker::needed(&request, &cwd) { return None; }
    let owned_args = args.to_vec();
    gpui_platform::application().run(move |cx: &mut App| {
        let (font, _) = ui::pick_mono(cx);
        let options = WindowOptions {
            app_id: Some("lenz".into()),
            focus: true,
            titlebar: Some(TitlebarOptions { title: Some("Lenz — open a repository".into()), ..Default::default() }),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            dock_icon_after_first_frame(window);
            ui::themes::install(cx);
            let dark = matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark);
            cx.update_global::<ui::themes::GuiThemes, _>(|g, _| { g.0.execute(crate::theme::ThemeCommand::NativeAppearance(dark)); });
            let chooser = cx.new(|cx| ui::repo_chooser::RepoChooser::new(crate::repo_picker::RepoPicker::new(cwd), font, cx));
            let focus = chooser.read(cx).focus_handle();
            window.focus(&focus, cx);
            chooser
        })
        .map_err(|e| { crate::diagnostics::failure("Open window", &e.to_string(), "Check the display connection, then retry."); cx.quit(); })
        .ok();
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        finish_on_terminate(cx, move || open_chosen(&owned_args));
        cx.activate(true);
    });
    Some(ExitCode::from(open_chosen(args)))
}

/// Start again inside the repository chosen in the chooser (`exec`, or a fresh process on macOS): 2 if that failed,
/// 0 when nothing was chosen or the new process started.
fn open_chosen(args: &[String]) -> u8 {
    let Some(root) = ui::repo_chooser::chosen() else { return 0 };
    let error = match std::env::current_exe() {
        // AppKit has already written this process off as quitting (the chooser closed through `terminate:`), so an
        // image exec'd into it starts but never shows a window: start the repository in a fresh process instead.
        #[cfg(target_os = "macos")]
        Ok(exe) => match std::process::Command::new(exe).args(args).current_dir(&root).spawn() {
            Ok(_) => return 0,
            Err(error) => error,
        },
        #[cfg(not(target_os = "macos"))]
        Ok(exe) => {
            use std::os::unix::process::CommandExt;
            std::process::Command::new(exe).args(args).current_dir(&root).exec()
        }
        Err(error) => error,
    };
    crate::diagnostics::failure("Open repository", &error.to_string(), "Run lenz inside the repository.");
    2
}

/// The Conflict List of the operation in progress at `root` (limited to `filter`) and its Change Set; `None` when
/// there is nothing to resolve.
fn conflict_list(root: &std::path::Path, filter: &[String]) -> Result<Option<(changeset::ChangeSet, ui::conflict_list::Snapshot)>, String> {
    let Some(snap) = ui::conflict_list::read(root, filter)?.filter(|s| !s.is_empty()) else { return Ok(None) };
    let set = ui::conflict_list::change_set(root, &ui::conflict_list::states(&snap, &Default::default()), &snap)?;
    Ok(Some((set, snap)))
}

/// Parses the arguments and lists the Change Set; any error is a startup error (exit 2).
/// The request (possibly reopened at the repository's last place, see [`crate::place::restore`]), its Change Set,
/// the place to restore, and the place file to record into.
type Loaded = (cli::Request, changeset::ChangeSet, Option<crate::place::Launch>);

/// Why the window could not open: a mistake in how it was started, or a failure worth a report.
enum LoadError { Usage(String), Failed(String) }

fn load(args: &[String]) -> Result<Loaded, LoadError> {
    let cwd = std::env::current_dir().map_err(|e| LoadError::Failed(format!("lenz: current directory: {e}")))?;
    let mut request = cli::parse(args, &cwd).map_err(|cli::UsageError(msg)| LoadError::Usage(msg))?;
    if matches!(request.launch, cli::Launch::Merge(cli::Merge::List(_))) && changeset::toplevel(&cwd).is_err() {
        return Err(LoadError::Usage("lenz merge: not inside a git repository (give BASE LOCAL REMOTE MERGED to merge plain files)".into()));
    }
    if matches!(request.spec, changeset::Spec::Git { .. }) && changeset::toplevel(&cwd).is_err() {
        return Err(LoadError::Usage("lenz: not inside a git repository (run it in one, or give two files or folders to compare)".into()));
    }
    let launch = crate::place::for_launch(&mut request, &cwd);
    let set = changeset::ChangeSet::load(&request.spec, &cwd).map_err(LoadError::Failed)?;
    Ok((request, set, launch))
}

/// The logo in the macOS Dock, set once the window's first frame has been presented: next-frame
/// callbacks run as a frame begins, so the second one runs after the first frame is on screen.
fn dock_icon_after_first_frame(window: &mut gpui::Window) {
    #[cfg(target_os = "macos")]
    window.on_next_frame(|window, _| window.on_next_frame(|_, _| {
        let started = std::time::Instant::now();
        let set = crate::dock_icon::install();
        if std::env::var_os("LENZ_TIMINGS").is_some() {
            eprintln!("lenz: dock icon {} in {:.1} ms", if set { "set" } else { "not set" }, started.elapsed().as_secs_f64() * 1000.0);
        }
    }));
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}
