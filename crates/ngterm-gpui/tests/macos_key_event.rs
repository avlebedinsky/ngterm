//! The terminal's native key for `NSEvent` key events built here, as `AppKit` would dispatch them: text a tool posts as
//! the event's Unicode string on a key that does not type it (text expanders, password managers' auto-type, the
//! conformance ax driver's `type_text`: key code 0, the A key), a hardware key whose characters differ from the layout's
//! (a dead key's composition), and a Control chord on the same key. It runs without a harness because the layout's input
//! source services want the main thread.
#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use std::time::Duration;
    // far beyond these calls; only a main thread that will never return reaches it
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        eprintln!("macos_key_event: the main thread hung");
        std::process::exit(1);
    });
    let failures: Vec<String> = [
        keys::a_posted_unicode_string_types_its_own_text_not_the_key_codes(),
        keys::a_hardware_key_types_the_layouts_text_whatever_its_characters(),
        keys::a_control_chord_keeps_the_layouts_text_for_its_key(),
    ]
    .into_iter()
    .flatten()
    .collect();
    for failure in &failures {
        eprintln!("macos_key_event: {failure}");
    }
    if !failures.is_empty() {
        std::process::exit(1);
    }
    println!("macos_key_event: 3 passed");
}

#[cfg(target_os = "macos")]
mod keys {
    use ngterm_gpui::terminal::input::{NativeKey, Source, event_source};
    use objc2::rc::Retained;
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
    use objc2_core_graphics::{CGEvent, CGEventField};
    use objc2_foundation::{NSPoint, NSString};

    /// The A key's XKB keycode, where key code 0 lands.
    const KEY_A: u32 = 38;

    /// A key-down event with `characters` on key code `code`, as a tool that posts text builds it: this process posts it.
    fn posted(characters: &str, code: u16, flags: NSEventModifierFlags) -> Result<Retained<NSEvent>, String> {
        let characters = NSString::from_str(characters);
        NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            NSEventType::KeyDown,
            NSPoint::new(0.0, 0.0),
            flags,
            0.0,
            0,
            None,
            &characters,
            &characters,
            false,
            code,
        )
        .ok_or_else(|| "AppKit built no key event".to_owned())
    }

    /// `event` as the keyboard delivers it: no process posted it.
    fn from_hardware(event: &NSEvent) -> Result<Retained<NSEvent>, String> {
        let quartz = event.CGEvent().ok_or("the key event has no Quartz event")?;
        CGEvent::set_integer_value_field(Some(&quartz), CGEventField::EventSourceUnixProcessID, 0);
        NSEvent::eventWithCGEvent(&quartz).ok_or_else(|| "AppKit built no event from the Quartz event".to_owned())
    }

    fn read(event: &NSEvent) -> Result<NativeKey, String> {
        // SAFETY: a live key event, on the main thread.
        match unsafe { event_source(std::ptr::from_ref(event).cast_mut().cast()) } {
            Source::Native(key) => Ok(key),
            other => Err(format!("key code {} has no native record: {other:?}", event.keyCode())),
        }
    }

    fn native(characters: &str, code: u16, flags: NSEventModifierFlags) -> Result<NativeKey, String> {
        let event = posted(characters, code, flags)?;
        read(&event)
    }

    fn expect_eq<T: PartialEq + std::fmt::Debug + ?Sized>(what: &str, got: &T, want: &T) -> Result<(), String> {
        if got == want {
            Ok(())
        } else {
            Err(format!("{what}: got {got:?}, want {want:?}"))
        }
    }

    pub fn a_posted_unicode_string_types_its_own_text_not_the_key_codes() -> Option<String> {
        let run = || -> Result<(), String> {
            for text in ["u", "Q", "é", "ж", "你", "🙂"] {
                let key = native(text, 0, NSEventModifierFlags::empty())?;
                expect_eq(&format!("the text of {text:?}"), key.text.as_str(), text)?;
                expect_eq(
                    &format!("the Option-free text of {text:?}"),
                    &key.without_option.as_deref(),
                    &Some(text),
                )?;
                expect_eq("the key's position", &key.keycode, &KEY_A)?;
            }
            let key = native("ж", 0, NSEventModifierFlags::Shift)?;
            expect_eq("the text with Shift", key.text.as_str(), "ж")?;
            expect_eq("Shift held", &key.modifiers.shift, &true)
        };
        run().err()
    }

    pub fn a_hardware_key_types_the_layouts_text_whatever_its_characters() -> Option<String> {
        let run = || -> Result<(), String> {
            let built = posted("ä", 0, NSEventModifierFlags::empty())?;
            let event = from_hardware(&built)?;
            let key = read(&event)?;
            let plain = key.unshifted.ok_or("the A key types no character")?.to_string();
            expect_eq("the text of the key", key.text.as_str(), plain.as_str())?;
            expect_eq(
                "the Option-free text of the key",
                &key.without_option.as_deref(),
                &Some(plain.as_str()),
            )
        };
        run().err()
    }

    pub fn a_control_chord_keeps_the_layouts_text_for_its_key() -> Option<String> {
        let run = || -> Result<(), String> {
            let key = native("\u{1}", 0, NSEventModifierFlags::Control)?;
            expect_eq("Control held", &key.modifiers.control, &true)?;
            let plain = key.unshifted.ok_or("the A key types no character")?.to_string();
            expect_eq(
                "the text before Control's transformation",
                key.text.as_str(),
                plain.as_str(),
            )
        };
        run().err()
    }
}
