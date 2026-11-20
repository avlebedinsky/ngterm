//! Following the desktop's light/dark appearance through the settings portal: the portal client against a fake
//! `org.freedesktop.portal.Desktop` on a private session bus (a current portal, one before `ReadOne`, none at all), the
//! GPUI build launched on the dark side and flipping to the light one when the portal says so, and the programs in its
//! panes told of each flip with a plain theme. Nothing here reaches the user's session bus or portal: every bus is the
//! sandbox's own.
//!
//! Linux only: the settings portal on a private D-Bus session bus, the app on a headless weston.

#![cfg(target_os = "linux")]

#[macro_use]
#[expect(
    dead_code,
    reason = "the sandbox serves every process test; this crate uses its bus and weston parts"
)]
mod support;

use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ngterm_gpui::appearance::{ColorScheme, KEY, NAMESPACE, PORTAL, PORTAL_PATH, Portal, SETTINGS_INTERFACE};
use serde_json::json;
use support::live::call;
use support::{DEADLINE, Sandbox, TestResult, have, lavapipe_icd, wait_for};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedValue, Value};

/// The portal's settings: the color scheme it holds.
struct Settings {
    scheme: Arc<Mutex<u32>>,
}

impl Settings {
    fn value(&self, namespace: &str, key: &str) -> zbus::fdo::Result<u32> {
        if namespace == NAMESPACE && key == KEY {
            Ok(*self.scheme.lock().unwrap_or_else(PoisonError::into_inner))
        } else {
            Err(zbus::fdo::Error::Failed("Requested setting not found".to_owned()))
        }
    }
}

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    fn read_one(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        Ok(OwnedValue::from(self.value(namespace, key)?))
    }

    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        legacy(self.value(namespace, key)?)
    }
}

/// A portal before version 2: `Read` only, its value wrapped in one more variant.
struct LegacySettings {
    scheme: u32,
}

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl LegacySettings {
    fn read(&self, namespace: &str, key: &str) -> zbus::fdo::Result<OwnedValue> {
        if namespace == NAMESPACE && key == KEY {
            legacy(self.scheme)
        } else {
            Err(zbus::fdo::Error::Failed("Requested setting not found".to_owned()))
        }
    }
}

fn legacy(scheme: u32) -> zbus::fdo::Result<OwnedValue> {
    OwnedValue::try_from(Value::Value(Box::new(Value::U32(scheme))))
        .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
}

/// A fake portal serving `iface` on the sandbox's bus.
fn serve<I: zbus::object_server::Interface>(sandbox: &Sandbox, iface: I) -> TestResult<Connection> {
    let connection = zbus::blocking::connection::Builder::address(sandbox.bus_address())?
        .name(PORTAL)?
        .serve_at(PORTAL_PATH, iface)?
        .build()?;
    Ok(connection)
}

/// The portal announces a change of another setting of the same namespace nobody follows: a u32 (`contrast` on),
/// which would read as `PreferDark` if it were taken for the color scheme.
fn announce_contrast(portal: &Connection) -> TestResult {
    portal.emit_signal(
        None::<()>,
        PORTAL_PATH,
        SETTINGS_INTERFACE,
        "SettingChanged",
        &(NAMESPACE, "contrast", Value::U32(1)),
    )?;
    Ok(())
}

/// The portal announces `scheme`.
fn announce(portal: &Connection, scheme: u32) -> TestResult {
    portal.emit_signal(
        None::<()>,
        PORTAL_PATH,
        SETTINGS_INTERFACE,
        "SettingChanged",
        &(NAMESPACE, KEY, Value::U32(scheme)),
    )?;
    Ok(())
}

fn client(sandbox: &Sandbox) -> TestResult<Portal> {
    Ok(Portal::over(sandbox.bus()?))
}

#[test]
fn the_portals_scheme_is_read_and_followed() -> TestResult {
    let sandbox = Sandbox::new()?;
    let scheme = Arc::new(Mutex::new(1));
    let portal = serve(
        &sandbox,
        Settings {
            scheme: Arc::clone(&scheme),
        },
    )?;
    let reader = client(&sandbox)?;
    assert_eq!(reader.color_scheme()?, ColorScheme::PreferDark);
    assert!(reader.is_dark());
    let (sender, received) = mpsc::channel();
    client(&sandbox)?.follow(move |scheme| {
        let _ = sender.send(scheme);
    })?;
    // the read made once subscribed
    assert_eq!(received.recv_timeout(DEADLINE)?, ColorScheme::PreferDark);
    *scheme.lock().unwrap() = 2;
    // another setting's change ahead of the scheme's would be received first
    announce_contrast(&portal)?;
    announce(&portal, 2)?;
    assert_eq!(received.recv_timeout(DEADLINE)?, ColorScheme::PreferLight);
    announce(&portal, 0)?;
    assert_eq!(received.recv_timeout(DEADLINE)?, ColorScheme::NoPreference);
    announce_contrast(&portal)?;
    assert!(
        received.recv_timeout(Duration::from_millis(200)).is_err(),
        "another setting's change is not followed"
    );
    Ok(())
}

#[test]
fn a_portal_before_read_one_is_read_through_read() -> TestResult {
    let sandbox = Sandbox::new()?;
    let _portal = serve(&sandbox, LegacySettings { scheme: 1 })?;
    assert_eq!(client(&sandbox)?.color_scheme()?, ColorScheme::PreferDark);
    Ok(())
}

#[test]
fn without_a_portal_the_desktop_reads_as_light() -> TestResult {
    let sandbox = Sandbox::new()?;
    let reader = client(&sandbox)?;
    assert!(reader.color_scheme().is_err());
    assert!(!reader.is_dark());
    Ok(())
}

/// A theme file of the user's: its background marks the side in the log.
fn write_theme(sandbox: &Sandbox, name: &str, background: &str) -> TestResult {
    let dir = sandbox.path("config/ghostty/themes");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join(name),
        format!("background = {background}\nforeground = #808080\n"),
    )?;
    Ok(())
}

/// The side the app's config was last fed on, through `debug.appearance`'s read.
fn applied_side(sandbox: &Sandbox) -> TestResult<String> {
    let reply = sandbox.request("debug.appearance")?;
    let reply: serde_json::Value = serde_json::from_str(&reply)?;
    Ok(reply["result"]["text"].as_str().unwrap_or_default().to_owned())
}

#[test]
fn the_app_starts_on_the_desktops_side_and_follows_a_flip() -> TestResult {
    if !have("weston") {
        skip!("no weston");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (set NGTERM_LAVAPIPE_ICD)");
    };
    let mut sandbox = Sandbox::new()?;
    let scheme = Arc::new(Mutex::new(1));
    let portal = serve(
        &sandbox,
        Settings {
            scheme: Arc::clone(&scheme),
        },
    )?;
    write_theme(&sandbox, "Paper", "#eeeeee")?;
    write_theme(&sandbox, "Ink", "#111111")?;
    std::fs::write(
        sandbox.path("state/settings.json"),
        r#"{"theme": "Paper", "darkTheme": "Ink", "followSystemAppearance": true}"#,
    )?;
    // a UI-test launch, so `debug.appearance` reads the side back
    sandbox.set_env("NGTERM_UITEST_FORCE_SIDEBAR_VISIBLE", "1");
    sandbox.start_weston()?;
    sandbox.use_software_rendering(&lavapipe);
    let primary = sandbox.start_gpui_primary("primary.log")?;
    assert_eq!(applied_side(&sandbox)?, "dark", "{}", sandbox.log("primary.log"));
    sandbox.wait_log("primary.log", "themes/Ink")?;
    *scheme.lock().unwrap() = 2;
    announce(&portal, 2)?;
    wait_for(|| applied_side(&sandbox).ok().filter(|side| side == "light"))
        .map_err(|error| format!("{error}\n{}", sandbox.log("primary.log")))?;
    sandbox.wait_log("primary.log", "themes/Paper")?;
    sandbox.terminate(primary)?;
    let (status, _) = sandbox.wait_exit(primary, Duration::from_secs(10))?;
    assert_eq!(status.code(), Some(0), "{status:?}");
    Ok(())
}

/// A session running a program that puts its terminal in raw mode, writes `request` to it and keeps every byte the
/// terminal sends back; the file that holds them.
fn probe(sandbox: &Sandbox, name: &str, request: &str) -> TestResult<(String, std::path::PathBuf)> {
    let received = sandbox.path(&format!("{name}.received"));
    let script = sandbox.path(&format!("{name}.sh"));
    std::fs::write(
        &script,
        format!(
            "stty raw -echo\nprintf '{request}'\nexec cat > '{}'\n",
            received.display()
        ),
    )?;
    let command = format!("/bin/sh {}", script.display());
    let id = call(sandbox, "session.new", None, json!({ "command": command }))?["id"]
        .as_str()
        .ok_or("session.new gave no id")?
        .to_owned();
    Ok((id, received))
}

/// Waits until `file` holds exactly `expected`.
fn wait_received(sandbox: &Sandbox, file: &std::path::Path, expected: &str) -> TestResult {
    wait_for(|| {
        std::fs::read_to_string(file)
            .ok()
            .filter(|received| received == expected)
    })
    .map(drop)
    .map_err(|error| {
        let received = std::fs::read_to_string(file).unwrap_or_default();
        format!(
            "{error}: {received:?}, expected {expected:?}\n{}",
            sandbox.log("primary.log")
        )
        .into()
    })
}

const DARK: &str = "\x1b[?997;1n";
const LIGHT: &str = "\x1b[?997;2n";

#[test]
fn with_a_plain_theme_a_flip_reaches_the_programs_in_the_panes() -> TestResult {
    if !have("weston") {
        skip!("no weston");
    }
    let Some(lavapipe) = lavapipe_icd() else {
        skip!("no lavapipe ICD (set NGTERM_LAVAPIPE_ICD)");
    };
    let mut sandbox = Sandbox::new()?;
    let scheme = Arc::new(Mutex::new(1));
    let portal = serve(
        &sandbox,
        Settings {
            scheme: Arc::clone(&scheme),
        },
    )?;
    write_theme(&sandbox, "Paper", "#eeeeee")?;
    std::fs::write(sandbox.path("state/settings.json"), r#"{"theme": "Paper"}"#)?;
    sandbox.start_weston()?;
    sandbox.use_software_rendering(&lavapipe);
    let primary = sandbox.start_gpui_primary("primary.log")?;
    // the query's answer shows the terminal took mode 2031 before the flip
    let (_, reports) = probe(&sandbox, "reports", "\\033[?2031h\\033[?996n")?;
    wait_received(&sandbox, &reports, DARK)?;
    *scheme.lock().unwrap() = 2;
    announce(&portal, 2)?;
    wait_received(&sandbox, &reports, &format!("{DARK}{LIGHT}"))?;
    // a pane opened after the flip answers its side, and without mode 2031 hears of no later flip
    let (quiet, answers) = probe(&sandbox, "answers", "\\033[?996n")?;
    wait_received(&sandbox, &answers, LIGHT)?;
    *scheme.lock().unwrap() = 1;
    announce(&portal, 1)?;
    wait_received(&sandbox, &reports, &format!("{DARK}{LIGHT}{DARK}"))?;
    call(&sandbox, "session.type", Some(&quiet), json!({ "text": "typed" }))?;
    wait_received(&sandbox, &answers, &format!("{LIGHT}typed"))?;
    sandbox.terminate(primary)?;
    let (status, _) = sandbox.wait_exit(primary, Duration::from_secs(10))?;
    assert_eq!(status.code(), Some(0), "{status:?}");
    Ok(())
}
