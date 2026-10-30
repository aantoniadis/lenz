use code_core::semantics::{installer, provider::{PROVIDERS, Settings}};

#[test]
fn server_install_recipes_are_curated_and_preserve_custom_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let bash = PROVIDERS.iter().find(|p| p.id == "bash-language-server").unwrap();
    let settings = Settings::default();
    let preview = installer::preview(bash, &settings, temp.path());
    assert!(preview.command.contains("bash-language-server@5.8.1"));
    assert!(preview.command.contains("--prefix"));
    assert!(!preview.command.contains("install -g"));
    let mut custom = settings.clone();
    custom.providers.entry(bash.id.into()).or_default().executable = Some("/custom/bash-lsp".into());
    assert!(installer::preview(bash, &custom, temp.path()).disabled.unwrap().contains("custom"));
    let java = PROVIDERS.iter().find(|p| p.id == "jdtls").unwrap();
    assert!(installer::preview(java, &settings, temp.path()).disabled.is_some());
}

#[cfg(unix)]
#[test]
fn server_install_streams_bounded_output_cancels_and_preserves_settings() {
    use std::{os::unix::fs::PermissionsExt, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    for (name, script) in [("node", "#!/bin/sh\nif [ \"$1\" = --version ]; then echo v24.0.0; else exec \"$@\"; fi\n"), ("npm", "#!/bin/sh\n(/bin/sleep 0.4; echo escaped > \"$3/escaped\") &\necho installing\nwait\n")] {
        let path = bin.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let bash = PROVIDERS.iter().find(|p| p.id == "bash-language-server").unwrap();
    let mut settings = Settings::default();
    settings.providers.entry(bash.id.into()).or_default().env.insert("PATH".into(), format!("{}:/usr/bin:/bin", bin.display()));
    let preview = installer::preview(bash, &settings, temp.path());
    assert!(preview.disabled.is_none(), "{:?}", preview.disabled);
    let cancel = Arc::new(AtomicBool::new(false));
    let start = Instant::now();
    let mut output = String::new();
    let result = installer::run(preview.plan.as_ref().unwrap(), &cancel, &mut |chunk| {
        output.push_str(chunk);
        if output.contains("\ninstalling\n") { cancel.store(true, Ordering::Relaxed); }
    });
    assert!(result.unwrap_err().contains("cancelled"));
    assert!(output.contains("\ninstalling\n"));
    assert!(start.elapsed() < Duration::from_secs(3));
    std::thread::sleep(Duration::from_millis(450));
    assert!(!preview.plan.as_ref().unwrap().directory.join("escaped").exists(), "the installer descendant survived cancellation");
}

#[cfg(unix)]
#[test]
fn server_install_uses_the_selected_node_for_npm_and_validates_before_persisting() {
    use std::{os::unix::fs::PermissionsExt, sync::atomic::AtomicBool};
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let node = bin.join("node");
    std::fs::write(&node, "#!/bin/sh\ncase \"$1\" in --version) echo v24.0.0;; --check) exit 0;; *) echo selected-runtime; exec \"$@\";; esac\n").unwrap();
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o700)).unwrap();
    let npm = bin.join("npm");
    std::fs::write(&npm, "#!/bin/sh\n[ \"$1\" = install ] || exit 9\nmkdir -p \"$3/node_modules/pyright/dist\" \"$3/node_modules/.bin\"\nprintf '%s' '{\"name\":\"pyright\",\"version\":\"1.1.414\"}' > \"$3/node_modules/pyright/package.json\"\nprintf '%s' 'console.log(1)' > \"$3/node_modules/pyright/dist/server.js\"\nln -s ../pyright/dist/server.js \"$3/node_modules/.bin/pyright-langserver\"\nchmod +x \"$3/node_modules/pyright/dist/server.js\"\necho installed\n").unwrap();
    std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o700)).unwrap();
    let pyright = PROVIDERS.iter().find(|p| p.id == "pyright").unwrap();
    let file = temp.path().join("providers.json");
    let mut settings = Settings::default();
    let entry = settings.providers.entry(pyright.id.into()).or_default();
    entry.node = Some(node.clone());
    entry.env.insert("PATH".into(), format!("{}:/usr/bin:/bin",bin.display()));
    entry.settings = Some(serde_json::json!({"pythonPath":"/custom/python"}));
    assert!(settings.save(&file));
    let preview = installer::preview(pyright, &settings, &temp.path().join("servers"));
    let plan = preview.plan.unwrap();
    let mut output = String::new();
    let lock = installer::run(&plan, &AtomicBool::new(false), &mut |chunk| output.push_str(chunk)).unwrap();
    assert!(output.contains("selected-runtime"), "{output}");
    let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    raw["future-setting"] = serde_json::json!({"keep":true});
    raw["providers"]["pyright"]["future"] = serde_json::json!(["keep"]);
    raw["providers"]["yaml-language-server"] = serde_json::json!({"settings":{"format":{"enable":false}}});
    std::fs::write(&file,serde_json::to_vec(&raw).unwrap()).unwrap();
    let contender=installer::run(&plan,&AtomicBool::new(false),&mut |_|{});
    assert!(contender.unwrap_err().contains("already running"),"validation must keep the lock until persistence");
    installer::persist(&plan, &file, &settings, &AtomicBool::new(false)).unwrap();
    drop(lock);
    let raw_saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(raw_saved["future-setting"],raw["future-setting"]);
    assert_eq!(raw_saved["providers"]["pyright"]["future"],raw["providers"]["pyright"]["future"]);
    assert_eq!(raw_saved["providers"]["yaml-language-server"],raw["providers"]["yaml-language-server"]);
    let saved = Settings::read(&file);
    assert_eq!(saved.providers[pyright.id].executable.as_ref(), Some(&plan.executable));
    assert_eq!(saved.providers[pyright.id].node.as_ref(),Some(&node));
    assert_eq!(saved.providers[pyright.id].settings, settings.providers[pyright.id].settings);
    let mut changed = saved.clone();
    changed.providers.get_mut(pyright.id).unwrap().args = Some(vec!["custom".into()]);
    assert!(changed.save(&file));
    assert!(installer::persist(&plan,&file,&settings,&AtomicBool::new(false)).unwrap_err().contains("changed"));
    assert_eq!(Settings::read(&file),changed);
    let mut script=std::fs::read_to_string(&npm).unwrap();
    script.push_str("\nchmod 600 \"$3/node_modules/pyright/dist/server.js\"\n");
    std::fs::write(&npm,script).unwrap();
    let why=installer::run(&plan,&AtomicBool::new(false),&mut |_|{}).unwrap_err();
    assert!(why.contains("executable"),"{why}");
    let script=std::fs::read_to_string(&npm).unwrap().replace("chmod 600","chmod 700").replace("1.1.414","0.0.0");
    std::fs::write(&npm,script).unwrap();
    assert!(installer::run(&plan,&AtomicBool::new(false),&mut |_|{}).unwrap_err().contains("version"));
}

#[cfg(unix)]
#[test]
fn server_install_bounds_and_sanitizes_subprocess_output() {
    use std::{os::unix::fs::PermissionsExt,sync::atomic::AtomicBool};
    let dir=tempfile::tempdir().unwrap();let bin=dir.path().join("bin");std::fs::create_dir(&bin).unwrap();
    for (name,script) in [("node","#!/bin/sh\nif [ \"$1\" = --version ]; then echo v24.0.0; else exec \"$@\"; fi\n"),("npm","#!/bin/sh\nprintf '\\033[31m'\n/usr/bin/head -c 131072 /dev/zero\nexit 1\n")] {
        let path=bin.join(name);std::fs::write(&path,script).unwrap();std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let provider=PROVIDERS.iter().find(|p|p.id=="bash-language-server").unwrap();let mut settings=Settings::default();
    settings.providers.entry(provider.id.into()).or_default().env.insert("PATH".into(),bin.display().to_string());
    let preview=installer::preview(provider,&settings,dir.path());let mut output=String::new();
    let result=installer::run(preview.plan.as_ref().unwrap(),&AtomicBool::new(false),&mut |chunk|output.push_str(chunk));
    assert!(result.is_err());assert!(output.len()<=installer::OUTPUT_LIMIT+512,"{}",output.len());
    assert!(!output.chars().any(|c|c.is_control()&&c!='\n'));
}

#[cfg(unix)]
fn lock_preview(root: &std::path::Path) -> installer::Preview {
    let provider=PROVIDERS.iter().find(|p|p.id=="bash-language-server").unwrap();
    let mut settings=Settings::default();
    settings.providers.entry(provider.id.into()).or_default().env.insert("PATH".into(),format!("{}:/usr/bin:/bin",root.join("bin").display()));
    installer::preview(provider,&settings,&root.join("servers"))
}

#[cfg(unix)]
#[test]
fn server_install_lock_child() {
    use std::{sync::{Arc,atomic::{AtomicBool,Ordering}},time::{Duration,Instant}};
    let Some(root)=std::env::var_os("LENZ_INSTALL_LOCK_TEST_DIR") else {return};let root=std::path::PathBuf::from(root);
    let cancel=Arc::new(AtomicBool::new(false));let signal=cancel.clone();let stop=root.join("cancel");
    let monitor=std::thread::spawn(move||{let start=Instant::now();while !stop.exists()&&start.elapsed()<Duration::from_secs(5){std::thread::sleep(Duration::from_millis(10));}signal.store(true,Ordering::Relaxed);});
    let result=installer::run(lock_preview(&root).plan.as_ref().unwrap(),&cancel,&mut |_|{});
    monitor.join().unwrap();assert!(result.unwrap_err().contains("cancelled"));
}

#[cfg(unix)]
#[test]
fn server_install_lock_excludes_an_independent_process_and_cancel_allows_retry() {
    use std::{os::unix::fs::PermissionsExt,sync::atomic::{AtomicBool,Ordering},time::{Duration,Instant}};
    let dir=tempfile::tempdir().unwrap();let bin=dir.path().join("bin");std::fs::create_dir(&bin).unwrap();
    for (name,script) in [("node","#!/bin/sh\nif [ \"$1\" = --version ]; then echo v24.0.0; else exec \"$@\"; fi\n"),("npm","#!/bin/sh\necho started >> \"$3/starts\"\necho LOCK_READY\nexec /bin/sleep 30\n")] {
        let path=bin.join(name);std::fs::write(&path,script).unwrap();std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let preview=lock_preview(dir.path());let plan=preview.plan.unwrap();
    let mut child=std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","server_install_lock_child","--nocapture"]).env("LENZ_INSTALL_LOCK_TEST_DIR",dir.path()).stdout(std::process::Stdio::null()).spawn().unwrap();
    let start=Instant::now();while !plan.directory.join("starts").exists()&&start.elapsed()<Duration::from_secs(3){std::thread::sleep(Duration::from_millis(10));}
    assert!(plan.directory.join("starts").exists(),"independent installer never started");
    let cancel=AtomicBool::new(false);
    let contender=installer::run(&plan,&cancel,&mut |chunk|{if chunk.contains("LOCK_READY"){cancel.store(true,Ordering::Relaxed);}});
    let starts=std::fs::read_to_string(plan.directory.join("starts")).unwrap().lines().count();
    std::fs::write(dir.path().join("cancel"),b"cancel").unwrap();assert!(child.wait().unwrap().success());
    assert!(contender.unwrap_err().contains("already running"));assert_eq!(starts,1,"the competing installer ran npm");
    let cancel=AtomicBool::new(false);
    let retry=installer::run(&plan,&cancel,&mut |chunk|{if chunk.contains("LOCK_READY"){cancel.store(true,Ordering::Relaxed);}});
    assert!(retry.unwrap_err().contains("cancelled"));
    assert_eq!(std::fs::read_to_string(plan.directory.join("starts")).unwrap().lines().count(),2);
}
