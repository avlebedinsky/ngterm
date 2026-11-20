//! The desktop's appearance reaching a deck's terminals with no config reload (upstream #961): a program in mode 2031
//! gets a report of the flip, and a later color scheme query answers the new side, whatever the theme. Each pane runs
//! a `/bin/sh` probe with no `NGTERM_*` or `AGTERM_*` variable inherited, writing what it reads into its own directory.

use super::*;

const PROBE: &str = concat!(
    "stty -icanon -echo min 1; printf '\\033[?2031hready'; ",
    "dd bs=1 count=9 of=\"$PROBE_DIR/report\" 2>/dev/null; ",
    "printf '\\033[?996n'; dd bs=1 count=9 of=\"$PROBE_DIR/query\" 2>/dev/null; exec cat"
);

/// Runs [`PROBE`] in every pane, each in `root/<pane token>`.
fn probe_launcher(root: std::path::PathBuf, launches: Arc<Mutex<Vec<Launch>>>) -> pane::Launcher {
    Rc::new(move |launch: &Launch| -> Spawn {
        launches.lock().unwrap().push(launch.clone());
        let token = launch
            .env
            .iter()
            .find_map(|(key, value)| (key == "NGTERM_PANE_ID").then(|| value.clone()))
            .expect("a pane token");
        let dir = root.join(token);
        std::fs::create_dir_all(&dir).unwrap();
        let mut command = PaneCommand::new("/bin/sh").arg("-c").arg(PROBE).env("PROBE_DIR", dir);
        for (key, _) in std::env::vars_os() {
            let name = key.to_string_lossy();
            if name.starts_with("NGTERM_") || name.starts_with("AGTERM_") {
                command = command.env_remove(key);
            }
        }
        Box::new(move |config: PaneConfig| Pane::spawn(&command, &config))
    })
}

fn wait_file(path: &Path, len: usize) -> Vec<u8> {
    let deadline = Instant::now() + DEADLINE;
    loop {
        let bytes = std::fs::read(path).unwrap_or_default();
        if bytes.len() >= len {
            return bytes;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {}", path.display());
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[gpui_kit::test]
fn a_flip_without_a_reload_reports_to_mode_2031_and_answers_later_queries(cx: &mut TestAppContext) {
    let probes = tempfile::tempdir().unwrap();
    let (library, window, ids) = three_sessions();
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let spawner = probe_launcher(probes.path().to_path_buf(), Arc::clone(&recorded));
    let mut h = harness_with(cx, core_of(library), window, recorded, spawner);
    let session = ids[0];
    h.until("the pane to start", |h| h.realized(session, StatusPane::Left));
    let token = h.token(session, StatusPane::Left);
    let shared = h.shared(session, &token);
    let pane = shared.pane().expect("started");
    assert_eq!(pane.color_scheme(), ColorScheme::Light, "the default config's side");
    h.until("the probe to turn mode 2031 on", |_| {
        pane.frame().frame.lines().iter().any(|line| line.contains("ready"))
    });

    h.deck
        .read_with(h.cx, |deck, _| deck.set_color_scheme(ColorScheme::Dark));
    let dir = probes.path().join(&token);
    assert_eq!(wait_file(&dir.join("report"), 9), b"\x1b[?997;1n", "the flip's report");
    assert_eq!(
        wait_file(&dir.join("query"), 9),
        b"\x1b[?997;1n",
        "the query answers dark"
    );
    assert_eq!(shared.config().color_scheme, ColorScheme::Dark);
}

#[test]
fn terminals_created_after_a_flip_start_on_its_side_overlays_included() {
    let state_dir = tempfile::tempdir().unwrap();
    let env = DeckEnv::new(
        PaneContext {
            socket: "/tmp/ngterm-test.sock".to_owned(),
            program_version: "1.2.3".to_owned(),
        },
        test_launcher(Arc::default()),
        TerminalConfig::default(),
        crate::state::detached_poster(),
        Watermarks {
            state_dir: state_dir.path().to_path_buf(),
            config_with: Rc::new(config_with),
        },
    );
    assert_eq!(
        env.config_for(Some("background = #102030\n")).color_scheme,
        ColorScheme::Light
    );
    env.set_color_scheme(ColorScheme::Dark);
    assert_eq!(env.config_for(None).color_scheme, ColorScheme::Dark);
    let overlaid = env.config_for(Some("background = #102030\n"));
    assert_eq!(overlaid.color_scheme, ColorScheme::Dark);
    assert_eq!(
        overlaid.theme.background,
        config_with("background = #102030\n").theme.background
    );
}
