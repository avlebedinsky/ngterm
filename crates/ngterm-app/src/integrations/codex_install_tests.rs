use super::*;

fn custom_home(home: &mut Home, root: &Path) {
    home.env
        .vars
        .insert("CODEX_HOME".into(), root.to_string_lossy().into_owned());
    home.write(".codex/config.toml", "default config");
}

fn default_is_untouched(home: &Home) {
    assert_eq!(home.read(".codex/config.toml"), "default config");
    assert!(!home.path(".codex/config.toml.bak").exists());
    assert!(!home.path(".codex/skills").exists());
}

#[test]
fn codex_home_resolves_relative_paths_without_shell_expansion() {
    let cwd = std::env::current_dir().unwrap();
    let resolve = |value| config::codex_home(value, Some(&cwd));
    assert_eq!(resolve(None), None);
    assert_eq!(resolve(Some("")), None);
    assert_eq!(resolve(Some("custom/../codex home")), Some(cwd.join("codex home")));
    assert_eq!(resolve(Some("~/codex")), Some(cwd.join("~").join("codex")));
    assert_eq!(resolve(Some(" ")), Some(cwd.join(" ")));
    assert_eq!(resolve(cwd.to_str()), Some(cwd.clone()));
    assert_eq!(config::codex_home(Some("relative"), None), None);
    assert_eq!(config::codex_home(Some("relative"), Some(Path::new("relative"))), None);
    assert_eq!(config::codex_home(cwd.to_str(), None), Some(cwd));
}

#[test]
fn custom_codex_hooks_merge_back_up_and_preserve_permissions() {
    let mut home = Home::new();
    let custom = home.path("custom codex");
    custom_home(&mut home, &custom);
    fs::create_dir_all(&custom).unwrap();
    let config = custom.join("config.toml");
    let original = "model = \"test\"\n";
    fs::write(&config, original).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let first = home.install_hooks(&no_opencode(&home), None);
    assert_eq!(first.codex, CodexResult::Merged);
    assert!(
        first
            .text()
            .contains(&format!("{}/config.toml", hooks::shell_path(&custom)))
    );
    assert_eq!(fs::read_to_string(custom.join("config.toml.bak")).unwrap(), original);
    #[cfg(unix)]
    for path in [&config, &custom.join("config.toml.bak")] {
        assert_eq!(files::posix_mode(path), Some(0o600));
    }
    let merged = fs::read_to_string(&config).unwrap();
    assert!(merged.contains(&hooks::shell_path(&hooks::script_dir(&home.env.home))));
    assert!(merged.contains(original));
    assert_eq!(
        home.install_hooks(&no_opencode(&home), None).codex,
        CodexResult::AlreadyConfigured
    );
    assert_eq!(fs::read_to_string(&config).unwrap(), merged);
    assert_eq!(fs::read_to_string(custom.join("config.toml.bak")).unwrap(), original);
    default_is_untouched(&home);
}

#[test]
fn a_custom_codex_skill_refresh_keeps_the_default_root() {
    let mut home = Home::new();
    let custom = home.path("custom codex");
    custom_home(&mut home, &custom);
    fs::create_dir_all(&custom).unwrap();
    let first_skill = skill::apply(&skill::plan(&home.env, None).unwrap());
    assert!(first_skill.skipped.is_empty());
    assert!(first_skill.installed.contains(&("Codex", custom.join("skills/ngterm"))));
    let installed = custom.join("skills/ngterm/SKILL.md");
    fs::write(&installed, format!("{}\nold", skill::MARKER)).unwrap();
    let repeat = skill::apply(&skill::plan(&home.env, None).unwrap());
    assert!(repeat.skipped.is_empty());
    assert_eq!(repeat.installed, first_skill.installed);
    assert_eq!(
        fs::read(&installed).unwrap(),
        fs::read(home.env.skill.as_ref().unwrap().join("SKILL.md")).unwrap()
    );
    default_is_untouched(&home);
}

#[test]
fn a_missing_custom_codex_root_is_reported_without_a_default_fallback() {
    let mut home = Home::new();
    let custom = home.path("absent codex");
    custom_home(&mut home, &custom);
    let hooks = home.install_hooks(&no_opencode(&home), None);
    assert_eq!(hooks.codex, CodexResult::NoCodex);
    assert!(
        hooks
            .text()
            .contains(&format!("No {} found (your CODEX_HOME)", hooks::shell_path(&custom)))
    );
    let skill = skill::apply(&skill::plan(&home.env, None).unwrap());
    assert_eq!(skill.installed, [("Claude Code", home.path(".claude/skills/ngterm"))]);
    assert_eq!(skill.skipped, [("Codex", format!("no {} found", custom.display()))]);
    assert!(!custom.exists());
    default_is_untouched(&home);
}

#[test]
fn a_custom_codex_skill_keeps_user_files_and_reports_the_destination() {
    let mut home = Home::new();
    let custom = home.path("custom codex");
    custom_home(&mut home, &custom);
    let skills = custom.join("skills");
    let destination = skills.join("ngterm");
    fs::create_dir_all(&destination).unwrap();
    fs::write(destination.join("SKILL.md"), "user skill").unwrap();
    let report = skill::apply(&skill::plan(&home.env, None).unwrap());
    assert!(report.installed.is_empty());
    assert_eq!(
        report.skipped,
        [(
            "Codex",
            format!(
                "{}: a different 'ngterm' skill is already there — left untouched",
                destination.display()
            )
        )]
    );
    assert_eq!(fs::read_to_string(destination.join("SKILL.md")).unwrap(), "user skill");
    fs::remove_dir_all(&skills).unwrap();
    fs::write(&skills, "not a directory").unwrap();
    let report = skill::apply(&skill::plan(&home.env, None).unwrap());
    assert!(report.installed.is_empty());
    assert_eq!(report.skipped.len(), 1);
    assert!(report.skipped[0].1.starts_with(&format!("{}: ", destination.display())));
    default_is_untouched(&home);
}

#[test]
fn custom_codex_hooks_keep_foreign_invalid_and_unreadable_configs() {
    let mut home = Home::new();
    let custom = home.path("custom codex");
    custom_home(&mut home, &custom);
    fs::create_dir_all(&custom).unwrap();
    let config = custom.join("config.toml");
    #[cfg(unix)]
    let target = {
        let target = home.path("linked.toml");
        symlink(&target, &config).unwrap();
        target
    };
    #[cfg(not(unix))]
    let target = config.clone();
    for (contents, expected) in [
        (
            b"[[hooks.Stop]]\ncommand = \"mine\"\n".as_slice(),
            CodexResult::HooksExist,
        ),
        (b"invalid = [".as_slice(), CodexResult::Unparseable),
        (b"\xff".as_slice(), CodexResult::Unreadable),
    ] {
        fs::write(&target, contents).unwrap();
        let report = home.install_hooks(&no_opencode(&home), None);
        assert_eq!(report.codex, expected);
        assert!(
            report
                .text()
                .contains(&format!("{}/config.toml", hooks::shell_path(&custom)))
        );
        assert_eq!(fs::read(&target).unwrap(), contents);
    }
    fs::write(&target, "model = \"test\"\n").unwrap();
    assert_eq!(home.install_hooks(&no_opencode(&home), None).codex, CodexResult::Merged);
    #[cfg(unix)]
    assert_eq!(fs::read_link(&config).unwrap(), target);
    default_is_untouched(&home);
}

#[test]
fn a_relative_codex_home_uses_the_process_working_directory() {
    let cwd = std::env::current_dir().unwrap();
    let root = tempfile::tempdir_in(&cwd).unwrap();
    let mut home = Home::new();
    let relative = root.path().strip_prefix(&cwd).unwrap().join("codex home");
    custom_home(&mut home, &relative);
    fs::create_dir_all(cwd.join(&relative)).unwrap();
    assert_eq!(home.env.codex_home(None), Some(cwd.join(&relative)));
    assert_eq!(home.install_hooks(&no_opencode(&home), None).codex, CodexResult::Merged);
    let report = skill::apply(&skill::plan(&home.env, None).unwrap());
    assert!(
        report
            .installed
            .contains(&("Codex", cwd.join(&relative).join("skills/ngterm")))
    );
    default_is_untouched(&home);
}

#[test]
fn an_empty_codex_home_uses_the_default_root() {
    let mut home = Home::new();
    home.env.vars.insert("CODEX_HOME".into(), String::new());
    home.mkdir(".codex");
    assert_eq!(home.env.codex_home(None), None);
    assert_eq!(home.install_hooks(&no_opencode(&home), None).codex, CodexResult::Merged);
    assert!(home.path(".codex/config.toml").exists());
    let report = skill::apply(&skill::plan(&home.env, None).unwrap());
    assert!(report.installed.contains(&("Codex", home.path(".codex/skills/ngterm"))));
}

#[cfg(not(windows))]
#[test]
fn the_login_shell_codex_home_applies_to_both_installers() {
    let mut home = Home::new();
    let custom = home.path("custom codex");
    custom_home(&mut home, &custom);
    fs::create_dir_all(&custom).unwrap();
    home.env.vars.remove("CODEX_HOME");
    let shell = home.path("shell");
    ngterm_host::fakes::write_executable(
        &shell,
        "#!/bin/sh\nexport CODEX_HOME='./custom codex'\ncd \"$HOME\"\nexec /bin/sh -c \"$2\"\n",
    )
    .unwrap();
    home.env
        .vars
        .insert("SHELL".into(), shell.to_string_lossy().into_owned());
    home.env
        .vars
        .insert("HOME".into(), home.env.home.to_string_lossy().into_owned());
    let probe = super::super::opencode::detect_with_timeout(
        &home.env.vars,
        &home.env.home_str(),
        crate::test_shell::CHILD_DEADLINE,
    )
    .expect("the login shell ran");
    assert_eq!(probe.codex_home, Some(custom.clone()));
    assert_eq!(
        home.install_hooks(&no_opencode(&home), Some(&probe)).codex,
        CodexResult::Merged
    );
    assert!(custom.join("config.toml").exists());
    let report = skill::apply(&skill::plan(&home.env, Some(&probe)).unwrap());
    assert!(report.installed.contains(&("Codex", custom.join("skills/ngterm"))));
    default_is_untouched(&home);
    home.env.vars.insert("CODEX_HOME".into(), String::new());
    assert_eq!(
        home.env.codex_home(Some(&probe)),
        None,
        "the process value wins, an empty one too"
    );
}

#[cfg(windows)]
#[test]
fn codex_home_variable_names_ignore_case_on_windows() {
    let mut home = Home::new();
    home.env
        .vars
        .insert("Codex_Home".into(), home.path("custom").to_string_lossy().into_owned());
    assert_eq!(home.env.codex_home(None), Some(home.path("custom")));
}

#[cfg(unix)]
#[test]
fn baked_wrappers_and_appended_rc_files_keep_their_modes() {
    let mut home = Home::new();
    let source = home.path("package");
    files::copy_tree(home.env.agent_status.as_ref().unwrap(), &source).unwrap();
    let wrappers = [(config::WRAPPER_NAME, 0o700), (config::CODEX_WRAPPER_NAME, 0o750)];
    for (wrapper, mode) in wrappers {
        fs::set_permissions(source.join(wrapper), fs::Permissions::from_mode(mode)).unwrap();
    }
    home.env.agent_status = Some(source);
    for rc in [".bashrc", ".zshrc", ".config/fish/config.fish"] {
        home.write(rc, "# user config\n");
        fs::set_permissions(home.path(rc), fs::Permissions::from_mode(0o600)).unwrap();
    }
    for _ in 0..2 {
        home.install_hooks(&no_opencode(&home), None);
        for rc in [".bashrc", ".zshrc", ".config/fish/config.fish"] {
            assert_eq!(files::posix_mode(&home.path(rc)), Some(0o600));
            assert!(home.read(rc).starts_with("# user config\n"));
            assert!(home.read(rc).contains(RC_MARKER_BEGIN));
        }
        for (wrapper, mode) in wrappers {
            let installed = hooks::script_dir(&home.env.home).join(wrapper);
            assert_eq!(files::posix_mode(&installed), Some(mode));
            assert!(
                fs::read_to_string(installed)
                    .unwrap()
                    .contains(config::NGTERMCTL_MARKER)
            );
        }
    }
}

#[test]
fn the_process_installer_uses_private_home_and_bundle_paths() {
    const CHILD: &str = "INSTALLER_ENV_TEST_CHILD";
    if let Ok(case) = std::env::var(CHILD) {
        assert_process_environment(&case);
        return;
    }
    let home = Home::new();
    let exe = std::env::current_exe().unwrap();
    for case in ["dev", "staged"] {
        let stage = home.path(case);
        let bin = stage.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let program = bin.join(exe.file_name().unwrap());
        fs::copy(&exe, &program).unwrap();
        if case == "staged" {
            for resource in ["agent-status", "skills/ngterm"] {
                fs::create_dir_all(stage.join("share/ngterm").join(resource)).unwrap();
            }
            fs::write(bin.join(super::super::cli::tool_file_name()), "bundled CLI").unwrap();
        }
        let mut command = std::process::Command::new(&program);
        for (key, _) in std::env::vars_os() {
            if super::super::adapter_tests::foreign_var(&key.to_string_lossy()) {
                command.env_remove(key);
            }
        }
        let output = command
            .args([
                "--exact",
                "integrations::tests::codex_install_tests::the_process_installer_uses_private_home_and_bundle_paths",
            ])
            .env(CHILD, case)
            .env("HOME", &home.env.home)
            .env("USERPROFILE", &home.env.home)
            .env("CODEX_HOME", "custom codex")
            .current_dir(&home.env.home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{case}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn assert_process_environment(case: &str) {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let env = Environment::process().unwrap();
    let bin = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    assert_eq!(env.home, home);
    assert_eq!(env.home_str(), home.to_string_lossy());
    assert_eq!(env.codex_home(None), Some(home.join("custom codex")));
    assert!(!env.vars.keys().any(|key| key.starts_with("NGTERM_")));
    let staged = case == "staged";
    assert_eq!(Environment::process_is_staged(), staged);
    let source = if staged {
        bin.parent().unwrap().join("share/ngterm")
    } else {
        resources()
    };
    assert_eq!(env.agent_status, Some(source.join("agent-status")));
    assert_eq!(env.skill, Some(source.join("skills/ngterm")));
    assert_eq!(
        env.bundled_cli,
        staged.then(|| bin.join(super::super::cli::tool_file_name()))
    );
}
