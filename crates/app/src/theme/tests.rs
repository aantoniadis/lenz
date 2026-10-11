use super::*;
const CUSTOM: &str = include_str!("fixtures/custom.toml");
#[test]
fn theme_import_and_presets_color_semantics() {
    let p = builtin("catppuccin").unwrap();
    assert_eq!((p.bg, p.fg, p.accent), (0x1e1e2e, 0xcdd6f4, 0x89b4fa));
    for id in ["catppuccin", "catppuccin-latte", "tokyo-night", "gruvbox"] {
        let p = builtin(id).unwrap();
        assert_ne!(p.block[0], p.block[1]);
        assert_ne!(p.inline[0], p.block[0]);
        assert_ne!(p.syntax.keyword, p.bg);
    }
    let loaded = parse_theme(CUSTOM, ThemeInputKind::User).unwrap();
    assert_eq!(loaded.name, "Custom café");
    assert_eq!((loaded.palette.bg, loaded.palette.fg), (0x1e1e2e, 0xcdd6f4));
}
#[test]
fn theme_overrides_are_strict_and_optional() {
    let text = format!(
        "{CUSTOM}\n[palette]\nbg = '#123456'\nblock = ['#010203','#040506','#070809','#0a0b0c']\n[syntax]\nkeyword = '#abcdef'\n"
    );
    let p = parse_theme(&text, ThemeInputKind::User).unwrap().palette;
    assert_eq!(p.bg, 0x123456);
    assert_eq!(p.block, [0x010203, 0x040506, 0x070809, 0x0a0b0c]);
    assert_eq!(p.syntax.keyword, 0xabcdef);
    for suffix in [
        "\n[palette]\nunknown = '#ffffff'",
        "\n[syntax]\nunknown = '#ffffff'",
        "\n[palette]\nedge = ['#ffffff']",
    ] {
        assert!(parse_theme(&format!("{CUSTOM}{suffix}"), ThemeInputKind::User).is_err());
    }
    assert!(parse_theme(&CUSTOM.replace("#1e1e2e", "red"), ThemeInputKind::User).is_err());
    assert!(parse_theme(&CUSTOM.replace("dark", "auto"), ThemeInputKind::User).is_err());
    assert!(
        parse_theme(
            &CUSTOM.replace("name = \"Custom café\"", ""),
            ThemeInputKind::User
        )
        .is_err()
    );
    assert!(
        parse_theme(
            &CUSTOM.replace("name = \"Custom café\"", ""),
            ThemeInputKind::Omarchy
        )
        .is_ok()
    );
}
#[test]
fn theme_reads_are_bounded_and_regular() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("theme.toml");
    std::fs::write(&path, CUSTOM).unwrap();
    assert!(load_theme(&path, ThemeInputKind::User).is_ok());
    std::fs::write(&path, vec![b' '; 65537]).unwrap();
    assert!(load_theme(&path, ThemeInputKind::User).is_err());
    assert!(load_theme(d.path(), ThemeInputKind::User).is_err());
    #[cfg(unix)]
    {
        std::fs::write(&path, CUSTOM).unwrap();
        let link = d.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(load_theme(&link, ThemeInputKind::User).is_ok());
        let fifo = d.path().join("fifo");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        assert!(load_theme(&fifo, ThemeInputKind::User).is_err());
    }
}
fn paths(dir: &std::path::Path) -> ThemePaths {
    ThemePaths {
        config_home: dir.join("config"),
        state_home: dir.join("state"),
    }
}
#[test]
fn theme_store_preserves_fields_and_override_precedence() {
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    std::fs::create_dir_all(p.config_home.join("lenz")).unwrap();
    let file = p.config_home.join("lenz/appearance.json");
    std::fs::write(&file, r#"{"version":1,"theme":"gruvbox","other":42}"#).unwrap();
    let (s, n) = read_selection(
        &p,
        &ThemeOverrides {
            theme: None,
            appearance: Some("light".into()),
        },
    );
    assert_eq!(s, ThemeSelection::Named("light".into()));
    assert!(n.is_empty());
    let (s, _) = read_selection(
        &p,
        &ThemeOverrides {
            theme: Some("system".into()),
            appearance: Some("light".into()),
        },
    );
    assert_eq!(s, ThemeSelection::System);
    let (s, n) = read_selection(
        &p,
        &ThemeOverrides {
            theme: Some("missing".into()),
            appearance: None,
        },
    );
    assert_eq!(s, ThemeSelection::Named("gruvbox".into()));
    assert_eq!(n.len(), 1);
    save_selection(&p, &ThemeSelection::Named("catppuccin".into())).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(v["other"], 42);
    assert_eq!(v["theme"], "catppuccin");
}
#[test]
fn theme_discovery_is_bounded_ordered_and_validated() {
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    let folder = p.config_home.join("lenz/themes");
    std::fs::create_dir_all(&folder).unwrap();
    for n in (0..260).rev() {
        std::fs::write(folder.join(format!("{n:03}.toml")), CUSTOM).unwrap();
    }
    let (entries, notices) = discover_themes(&p);
    assert_eq!(entries.len(), 256);
    assert_eq!(entries[0].id, "user:000");
    assert_eq!(entries.last().unwrap().id, "user:255");
    assert!(!notices.is_empty());
    assert!(!valid_id("user:../secret"));
    assert!(!valid_id("user:"));
    assert!(valid_id("user:café"));
}
#[test]
fn theme_controller_revisions_and_fixed_selection() {
    let mut c = ThemeController::new(ThemeSelection::System, builtin_entries(), true, None);
    let before = c.snapshot();
    let mut theme = parse_theme(CUSTOM, ThemeInputKind::User).unwrap();
    c.system(Some(theme.clone()));
    let now = c.snapshot();
    assert_ne!(now.revision, before.revision);
    theme.name = "renamed".into();
    c.system(Some(theme));
    assert_eq!(c.snapshot().revision, now.revision);
    assert_eq!(c.snapshot().source, "renamed");
    c.select(ThemeSelection::Named("gruvbox".into())).unwrap();
    let fixed = c.snapshot();
    c.system(None);
    assert_eq!(c.snapshot().revision, fixed.revision);
    assert_eq!(c.snapshot().palette.bg, 0x282828);
    c.select(ThemeSelection::Named("gruvbox".into())).unwrap();
    assert_eq!(c.snapshot().revision, fixed.revision);
    c.select(ThemeSelection::System).unwrap();
    assert_eq!(c.snapshot().palette.bg, 0x2b2b2b);
}
#[test]
fn theme_invalid_settings_are_untouched_and_atomic_saves_are_complete() {
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    std::fs::create_dir_all(p.config_home.join("lenz")).unwrap();
    let f = p.config_home.join("lenz/appearance.json");
    std::fs::write(&f, r#"{"version":9,"theme":"light"}"#).unwrap();
    let (_, n) = read_selection(
        &p,
        &ThemeOverrides {
            theme: None,
            appearance: None,
        },
    );
    assert!(!n.is_empty());
    assert!(std::fs::read_to_string(&f).unwrap().contains("9"));
    save_selection(&p, &ThemeSelection::System).unwrap();
    let a = p.clone();
    let t = std::thread::spawn(move || {
        save_selection(&a, &ThemeSelection::Named("light".into())).unwrap()
    });
    save_selection(&p, &ThemeSelection::Named("gruvbox".into())).unwrap();
    t.join().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(f).unwrap()).unwrap();
    assert_eq!(v["version"], 1);
    assert!(["light", "gruvbox"].contains(&v["theme"].as_str().unwrap()));
}
fn wait_theme(service: &mut ThemeService, bg: u32) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < until {
        while let Ok(update) = service.updates().try_recv() {
            service.apply(update);
        }
        if service.snapshot().palette.bg == bg {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("did not update: {}", service.snapshot().source);
}
fn settle_initial_scan(service: &mut ThemeService) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        assert!(
            std::time::Instant::now() < until,
            "initial scan not delivered"
        );
        if let Ok(update) = service.updates().try_recv() {
            let scanned = matches!(update, ThemeUpdate::System { .. });
            service.apply(update);
            if scanned {
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
#[cfg(target_os = "linux")]
#[test]
fn theme_staged_replacement_burst_retains_last_valid_palette() {
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    let current = p.state_home.join("omarchy/current");
    fs::create_dir_all(current.join("theme")).unwrap();
    fs::write(current.join("theme/colors.toml"), CUSTOM).unwrap();
    let mut s = ThemeService::new(p, ThemeOverrides::default(), true);
    settle_initial_scan(&mut s);
    let writer = thread::spawn(move || {
        fs::rename(current.join("theme"), current.join("previous")).unwrap();
        thread::sleep(Duration::from_millis(10));
        fs::create_dir(current.join("theme")).unwrap();
        thread::sleep(Duration::from_millis(15));
        for n in 0..8 {
            fs::write(current.join(format!("theme/app-{n}.conf")), "staging").unwrap();
            thread::sleep(Duration::from_millis(15));
        }
        fs::write(
            current.join("theme/colors.toml"),
            CUSTOM.replace("#1e1e2e", "#123456"),
        )
        .unwrap();
        fs::write(current.join("theme.name"), "final replacement").unwrap();
    });
    let until = Instant::now() + Duration::from_secs(3);
    let mut history = Vec::new();
    while Instant::now() < until {
        while let Ok(update) = s.updates().try_recv() {
            let effect = s.apply(update);
            history.push((s.snapshot().palette.bg, effect.notice));
        }
        if s.snapshot().source == "final replacement" {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    writer.join().unwrap();
    assert_eq!(s.snapshot().source, "final replacement");
    assert!(!history.is_empty());
    assert!(
        history
            .iter()
            .all(|(bg, notice)| [0x1e1e2e, 0x123456].contains(bg) && notice.is_none()),
        "palette history: {history:?}"
    );
}
#[cfg(target_os = "linux")]
#[test]
fn theme_late_creation_below_deep_missing_xdg_roots() {
    let d = tempfile::tempdir().unwrap();
    let deep = (0..15).fold(d.path().to_owned(), |p, n| p.join(format!("level-{n}")));
    let p = paths(&deep);
    let mut s = ThemeService::new(p.clone(), ThemeOverrides::default(), true);
    settle_initial_scan(&mut s);
    let current = p.state_home.join("omarchy/current/theme");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(current.join("colors.toml"), CUSTOM).unwrap();
    wait_theme(&mut s, 0x1e1e2e);
}
#[cfg(target_os = "linux")]
#[test]
fn theme_live_omarchy_replacement_repair_and_manual_override() {
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    let current = p.state_home.join("omarchy/current");
    std::fs::create_dir_all(current.join("theme")).unwrap();
    std::fs::write(current.join("theme/colors.toml"), CUSTOM).unwrap();
    std::fs::write(current.join("theme.name"), "catppuccin").unwrap();
    let mut s = ThemeService::new(p.clone(), ThemeOverrides::default(), true);
    assert_eq!(s.snapshot().palette.bg, 0x1e1e2e);
    std::thread::sleep(std::time::Duration::from_millis(60));
    let next = current.join("next");
    std::fs::create_dir(&next).unwrap();
    std::fs::write(
        next.join("colors.toml"),
        CUSTOM.replace("#1e1e2e", "#123456"),
    )
    .unwrap();
    std::fs::remove_dir_all(current.join("theme")).unwrap();
    std::fs::rename(next, current.join("theme")).unwrap();
    wait_theme(&mut s, 0x123456);
    std::thread::sleep(std::time::Duration::from_millis(500));
    while let Ok(update) = s.updates().try_recv() {
        s.apply(update);
    }
    std::fs::write(
        current.join("theme/colors.toml"),
        CUSTOM.replace("#1e1e2e", "#654321"),
    )
    .unwrap();
    wait_theme(&mut s, 0x654321);
    s.execute(ThemeCommand::Select(ThemeSelection::Named(
        "gruvbox".into(),
    )));
    std::fs::write(current.join("theme/colors.toml"), "broken").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(450));
    while let Ok(u) = s.updates().try_recv() {
        s.apply(u);
    }
    assert_eq!(s.snapshot().palette.bg, 0x282828);
    std::fs::write(current.join("theme/colors.toml"), CUSTOM).unwrap();
    s.execute(ThemeCommand::Select(ThemeSelection::System));
    wait_theme(&mut s, 0x1e1e2e);
    drop(s);
    let (saved, _) = read_selection(&p, &ThemeOverrides::default());
    assert_eq!(saved, ThemeSelection::System);
}
#[cfg(target_os = "linux")]
#[test]
fn theme_live_legacy_late_creation_and_stale_results() {
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    let mut s = ThemeService::new(p.clone(), ThemeOverrides::default(), true);
    let current = p.config_home.join("omarchy/current");
    std::fs::create_dir_all(current.join("theme")).unwrap();
    std::fs::write(current.join("theme/colors.toml"), CUSTOM).unwrap();
    wait_theme(&mut s, 0x1e1e2e);
    s.execute(ThemeCommand::Select(ThemeSelection::Named("light".into())));
    s.apply(ThemeUpdate::System {
        request: 0,
        theme: Some(parse_theme(CUSTOM, ThemeInputKind::User).unwrap()),
        notice: None,
    });
    assert_eq!(s.snapshot().palette.bg, 0xffffff);
}
#[test]
fn theme_custom_selection_revalidates_and_missing_saved_id_is_retained() {
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    save_selection(&p, &ThemeSelection::Named("user:missing".into())).unwrap();
    let mut s = ThemeService::new(p.clone(), ThemeOverrides::default(), true);
    assert_eq!(
        s.snapshot().selection,
        ThemeSelection::Named("user:missing".into())
    );
    assert_eq!(s.snapshot().palette.bg, 0x2b2b2b);
    assert!(!s.take_notices().is_empty());
    let folder = p.config_home.join("lenz/themes");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("missing.toml"), CUSTOM).unwrap();
    s.execute(ThemeCommand::OpenChooser);
    wait_theme(&mut s, 0x1e1e2e);
    std::fs::remove_file(folder.join("missing.toml")).unwrap();
    s.execute(ThemeCommand::Select(ThemeSelection::Named(
        "user:missing".into(),
    )));
    let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < until {
        if let Ok(update) = s.updates().try_recv() {
            let e = s.apply(update);
            if e.notice.is_some() {
                wait_theme(&mut s, 0x2b2b2b);
                assert_eq!(
                    s.snapshot().selection,
                    ThemeSelection::Named("user:missing".into())
                );
                drop(s);
                assert_eq!(
                    read_selection(&p, &ThemeOverrides::default()).0,
                    ThemeSelection::Named("user:missing".into())
                );
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("missing theme not reported");
}

#[cfg(target_os = "linux")]
#[test]
fn theme_failed_custom_selection_keeps_following_system() {
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    let current = p.state_home.join("omarchy/current/theme");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(current.join("colors.toml"), CUSTOM).unwrap();
    let mut s = ThemeService::new(p, ThemeOverrides::default(), true);
    s.execute(ThemeCommand::Select(ThemeSelection::Named(
        "user:removed".into(),
    )));
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        assert!(std::time::Instant::now() < until);
        if let Ok(u) = s.updates().try_recv()
            && s.apply(u).notice.is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(s.snapshot().selection, ThemeSelection::System);
    std::fs::write(
        current.join("colors.toml"),
        CUSTOM.replace("#1e1e2e", "#123456"),
    )
    .unwrap();
    wait_theme(&mut s, 0x123456);
    std::fs::remove_dir_all(current).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut notice = false;
    while std::time::Instant::now() < until {
        while let Ok(u) = s.updates().try_recv() {
            notice |= s.apply(u).notice.is_some();
        }
        if s.snapshot().palette.bg == 0x2b2b2b {
            assert!(notice, "confirmed removal should explain fallback");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("removal not delivered");
}
#[cfg(target_os = "linux")]
#[test]
fn theme_removed_custom_registry_resumes_live_fallback() {
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    let folder = p.config_home.join("lenz/themes");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("chosen.toml"), CUSTOM).unwrap();
    let current = p.state_home.join("omarchy/current/theme");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(
        current.join("colors.toml"),
        CUSTOM.replace("#1e1e2e", "#123456"),
    )
    .unwrap();
    let mut s = ThemeService::new(p, ThemeOverrides::default(), true);
    s.execute(ThemeCommand::Select(ThemeSelection::Named(
        "user:chosen".into(),
    )));
    wait_theme(&mut s, 0x1e1e2e);
    std::fs::remove_file(folder.join("chosen.toml")).unwrap();
    s.execute(ThemeCommand::OpenChooser);
    wait_theme(&mut s, 0x123456);
    std::fs::write(
        current.join("colors.toml"),
        CUSTOM.replace("#1e1e2e", "#654321"),
    )
    .unwrap();
    wait_theme(&mut s, 0x654321);
    assert_eq!(
        s.snapshot().selection,
        ThemeSelection::Named("user:chosen".into())
    );
}
#[test]
fn theme_notification_errors_request_reported_polling_fallback() {
    let mut state = super::service::Pending::default();
    state.follow = true;
    let pending = std::sync::Arc::new(std::sync::Mutex::new(state));
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    super::watch::on_event(
        &["/a/current".into(), "/b/current".into()],
        &pending,
        &tx,
        Err(notify::Error::generic("backend unavailable")),
    );
    let state = pending.lock().unwrap();
    assert!(state.reload && state.watch_failed);
    assert!(rx.try_recv().is_ok());
}
#[test]
fn theme_explicit_current_choice_supersedes_override_and_repairs_invalid_settings() {
    for override_theme in [true, false] {
        let d = tempfile::tempdir().unwrap();
        let p = paths(d.path());
        let chosen = if override_theme {
            save_selection(&p, &ThemeSelection::Named("light".into())).unwrap();
            ThemeSelection::Named("darcula".into())
        } else {
            std::fs::create_dir_all(p.config_home.join("lenz")).unwrap();
            std::fs::write(
                p.config_home.join("lenz/appearance.json"),
                r#"{"version":9,"theme":"light"}"#,
            )
            .unwrap();
            ThemeSelection::System
        };
        let overrides = ThemeOverrides {
            theme: override_theme.then(|| "darcula".into()),
            appearance: None,
        };
        let mut s = ThemeService::new(p.clone(), overrides, true);
        assert_eq!(s.snapshot().selection, chosen);
        s.execute(ThemeCommand::Select(chosen.clone()));
        drop(s);
        assert_eq!(read_selection(&p, &ThemeOverrides::default()).0, chosen);
        let json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(p.config_home.join("lenz/appearance.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(json["version"], 1);
    }
}

#[test]
fn theme_read_notifications_do_not_trigger_more_reads() {
    use std::sync::{Arc, Mutex, mpsc};
    let d = tempfile::tempdir().unwrap();
    let paths = paths(d.path());
    let targets = paths.omarchy();
    let mut state = super::service::Pending::default();
    state.follow = true;
    let pending = Arc::new(Mutex::new(state));
    let (wake, rx) = mpsc::sync_channel(1);
    for access in [
        notify::event::AccessKind::Read,
        notify::event::AccessKind::Open(notify::event::AccessMode::Read),
        notify::event::AccessKind::Close(notify::event::AccessMode::Read),
    ] {
        super::watch::on_event(
            &targets,
            &pending,
            &wake,
            Ok(notify::Event::new(notify::EventKind::Access(access))
                .add_path(targets[0].join("theme/colors.toml"))),
        );
        assert!(
            !pending.lock().unwrap().reload,
            "access events must not schedule I/O"
        );
        assert!(rx.try_recv().is_err());
    }
    super::watch::on_event(
        &targets,
        &pending,
        &wake,
        Ok(
            notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any))
                .add_path(targets[0].join("theme/colors.toml")),
        ),
    );
    assert!(pending.lock().unwrap().reload);
    assert!(rx.try_recv().is_ok());
}

#[cfg(target_os = "linux")]
#[test]
fn theme_deferred_service_resolves_before_start_and_converges_on_activation() {
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    let folder = p.state_home.join("omarchy/current/theme");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("colors.toml"), CUSTOM).unwrap();
    let mut s = ThemeService::new_deferred(p.clone(), ThemeOverrides::default(), true);
    assert_eq!(s.snapshot().palette.bg, 0x1e1e2e);
    assert!(s.updates().try_recv().is_err());
    std::fs::write(
        folder.join("colors.toml"),
        CUSTOM.replace("#1e1e2e", "#123456"),
    )
    .unwrap();
    assert_eq!(s.snapshot().palette.bg, 0x1e1e2e);
    s.start();
    wait_theme(&mut s, 0x123456);
    s.execute(ThemeCommand::Select(ThemeSelection::Named("light".into())));
    drop(s);
    assert_eq!(
        read_selection(&p, &ThemeOverrides::default()).0,
        ThemeSelection::Named("light".into())
    );
}

#[cfg(target_os = "linux")]
#[test]
fn theme_live_symlink_swap_rewatches_the_new_target() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let p = paths(d.path());
    let current = p.state_home.join("omarchy/current");
    std::fs::create_dir_all(&current).unwrap();
    for (name, color) in [("first", "#123456"), ("second", "#654321")] {
        let dir = d.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("colors.toml"), CUSTOM.replace("#1e1e2e", color)).unwrap();
    }
    symlink(d.path().join("first"), current.join("theme")).unwrap();
    let mut s = ThemeService::new(p, ThemeOverrides::default(), true);
    assert_eq!(s.snapshot().palette.bg, 0x123456);
    symlink(d.path().join("second"), current.join("next")).unwrap();
    std::fs::rename(current.join("next"), current.join("theme")).unwrap();
    wait_theme(&mut s, 0x654321);
    std::fs::write(
        d.path().join("second/colors.toml"),
        CUSTOM.replace("#1e1e2e", "#987654"),
    )
    .unwrap();
    wait_theme(&mut s, 0x987654);
}

#[test]
fn raised_surfaces_stand_out_in_every_builtin_theme() {
    let contrast = |a: u32, b: u32| { let (x, y) = (luminance(a), luminance(b)); (x.max(y) + 0.05) / (x.min(y) + 0.05) };
    for (id, _) in presets::BUILTINS {
        let p = builtin(id).unwrap();
        let (bg, border) = p.raised();
        for under in [p.header_bg, p.status_bg, p.bg] {
            assert!(contrast(border, under) >= 1.8, "{id}: popup edge {border:06x} blends with {under:06x}");
            if luminance(p.bg) < 0.4 { assert!(contrast(bg, under) >= 1.1, "{id}: popup {bg:06x} blends with {under:06x}"); }
        }
        assert!(contrast(border, bg) >= 2.0, "{id}: border {border:06x} on {bg:06x}");
        assert!(contrast(p.fg, bg) >= 4.5, "{id}: text {:06x} on {bg:06x}", p.fg);
    }
}

#[test]
fn extras_colours_follow_the_hue_of_each_kind() {
    use crate::markdown::CalloutKind;
    for p in [&DARCULA, &LIGHT] {
        assert_ne!(callout_color(p, CalloutKind::Warning), callout_color(p, CalloutKind::Note));
        assert_eq!(callout_color(p, CalloutKind::Quote), p.dim);
        assert_ne!(highlight_bg(p), p.bg);
        let (hue, _) = hue_saturation(highlight_bg(p));
        assert!((25. ..75.).contains(&hue), "highlights are amber, never red like a removal: {:x}", highlight_bg(p));
        assert_eq!(callout_icon(CalloutKind::Warning), "⚠");
        let (hue, saturation) = hue_saturation(removed_color(p));
        assert!(saturation > 0.25 && !(30. ..330.).contains(&hue), "removed text is red even where the theme greys deletions: {:x}", removed_color(p));
    }
}
#[test]
fn preview_resolves_a_palette_without_selecting_or_saving() {
    use std::time::{Duration, Instant};
    let d = tempfile::tempdir().unwrap();
    let mut s = ThemeService::new(paths(d.path()), ThemeOverrides::default(), true);
    settle_initial_scan(&mut s);
    s.execute(ThemeCommand::OpenChooser);
    let until = Instant::now() + Duration::from_secs(5);
    while s.entries().len() < 2 && Instant::now() < until {
        while let Ok(update) = s.updates().try_recv() { s.apply(update); }
        std::thread::sleep(Duration::from_millis(2));
    }
    let before = s.snapshot();
    let other = s.entries().iter().find(|e| *e.palette != *before.palette).expect("a theme that looks different").clone();
    assert_eq!(s.preview(&other.id).map(|p| *p), Some(*other.palette));
    assert!(s.preview("system").is_some(), "following the system resolves too");
    let after = s.snapshot();
    assert_eq!((after.selection, after.revision), (before.selection, before.revision), "previewing changes nothing");
}

#[test]
fn changed_words_keep_syntax_colors_that_stay_readable_on_their_highlight() {
    use crate::highlight::Capture::*;
    let (mut kept, mut total) = (0, 0);
    for entry in builtin_entries() {
        let p = &entry.palette;
        for fill in p.inline {
            for capture in [Keyword, String, Comment, Number, Function, Type, Macro, Constant, Attribute] {
                let color = p.syntax_color(capture);
                let chosen = p.readable_on(color, fill);
                assert!(chosen == color || chosen == p.fg, "{}: only the syntax color or the plain foreground", entry.id);
                if chosen == color {
                    assert!(super::contrast(color, fill) >= 3.0, "{}: {capture:?} kept at low contrast", entry.id);
                    kept += 1;
                }
                total += 1;
            }
        }
    }
    println!("syntax colors kept on changed-word highlights: {kept} of {total}");
    assert!(kept * 2 > total, "most syntax colors read on the highlights: {kept} of {total}");
}
