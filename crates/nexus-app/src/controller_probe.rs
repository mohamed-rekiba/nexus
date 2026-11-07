//! The game controllers, from SDL3, for emulators that map controllers by SDL (Lichii). It
//! runs in a child process (`nexus --probe-controllers`), so a broken driver can't take the
//! launcher down, and prints one line per controller: `controller<TAB>guid<TAB>name<TAB>mapping`.
//!
//! SDL3 is loaded while it runs (`libSDL3.so.0` on Linux), not linked: without it there are no
//! controllers, and games keep the keyboard. It sets the hints Lichii's `SDLDriver` sets, so a
//! controller has the GUID Lichii looks it up by (SDL gives a controller another GUID when
//! another of its drivers reads it).

use std::ffi::{CStr, c_char, c_int, c_void};
use std::time::Duration;

use nexus_api::Controller;

/// SDL3's library, by system.
const LIBRARY: &str = if cfg!(windows) {
    "SDL3.dll"
} else if cfg!(target_os = "macos") {
    "libSDL3.0.dylib"
} else {
    "libSDL3.so.0"
};

/// The hints Lichii's `SDLDriver` sets before SDL starts, with its default settings (its own
/// Joy-Con driver on, its Pro Controller driver off). The HIDAPI ones choose the driver that
/// reads a controller, and so its GUID.
const HINTS: [(&str, &str); 9] = [
    ("SDL_APP_NAME", "Eden"),
    ("SDL_JOYSTICK_HIDAPI_STEAM", "1"),
    ("SDL_GAMECONTROLLER_SENSOR_FUSION", "1"),
    ("SDL_JOYSTICK_ENHANCED_REPORTS", "1"),
    ("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1"),
    ("SDL_JOYSTICK_HIDAPI_JOY_CONS", "0"),
    ("SDL_JOYSTICK_HIDAPI_SWITCH", "1"),
    ("SDL_JOYSTICK_HIDAPI_SWITCH_PLAYER_LED", "1"),
    ("SDL_JOYSTICK_HIDAPI_XBOX", "0"),
];

/// `SDL_INIT_GAMEPAD`, which starts the joysticks too.
const INIT_GAMEPAD: u32 = 0x2000;

/// SDL's `SDL_GUID`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data: [u8; 16],
}

/// The GUID as Eden and its forks write it: the bytes in order, in hex, with the two of the
/// name's checksum (bytes 2 and 3) zeroed.
fn guid_text(mut guid: [u8; 16]) -> String {
    guid[2] = 0;
    guid[3] = 0;
    guid.iter().map(|b| format!("{b:02x}")).collect()
}

/// The inputs of an SDL mapping, without the GUID and name in front.
fn inputs(mapping: &str) -> String {
    mapping.splitn(3, ',').nth(2).unwrap_or("").trim_end_matches(',').to_owned()
}

/// Prints the controllers. `false` when SDL3 isn't there or doesn't start.
pub fn print() -> bool {
    match probe() {
        Some(found) => {
            for (guid, name, mapping) in found {
                let clean = |s: &str| s.replace(['\t', '\n'], " ");
                println!("controller\t{guid}\t{}\t{}", clean(&name), clean(&mapping));
            }
            true
        }
        None => false,
    }
}

type SetHint = unsafe extern "C" fn(*const c_char, *const c_char) -> bool;
type Init = unsafe extern "C" fn(u32) -> bool;
type Pump = unsafe extern "C" fn();
type Gamepads = unsafe extern "C" fn(*mut c_int) -> *mut u32;
type NameFor = unsafe extern "C" fn(u32) -> *const c_char;
type GuidFor = unsafe extern "C" fn(u32) -> Guid;
type MappingFor = unsafe extern "C" fn(u32) -> *mut c_char;
type Free = unsafe extern "C" fn(*mut c_void);
type Quit = unsafe extern "C" fn();

/// Each controller's GUID, name and mapping, in SDL's order.
#[allow(unsafe_code)] // SDL3's C functions, with the signatures of SDL 3.2's headers; each pointer SDL returns is read before SDL_free and SDL_Quit.
fn probe() -> Option<Vec<(String, String, String)>> {
    let library = unsafe { libloading::Library::new(LIBRARY) }.ok()?;
    let get =
        |name: &[u8]| -> Option<*const ()> { unsafe { library.get::<*const ()>(name) }.ok().map(|symbol| *symbol) };
    unsafe {
        let set_hint: SetHint = std::mem::transmute(get(b"SDL_SetHint\0")?);
        let init: Init = std::mem::transmute(get(b"SDL_Init\0")?);
        let pump: Pump = std::mem::transmute(get(b"SDL_PumpEvents\0")?);
        let gamepads: Gamepads = std::mem::transmute(get(b"SDL_GetGamepads\0")?);
        let name_for: NameFor = std::mem::transmute(get(b"SDL_GetGamepadNameForID\0")?);
        let guid_for: GuidFor = std::mem::transmute(get(b"SDL_GetJoystickGUIDForID\0")?);
        let mapping_for: MappingFor = std::mem::transmute(get(b"SDL_GetGamepadMappingForID\0")?);
        let free: Free = std::mem::transmute(get(b"SDL_free\0")?);
        let quit: Quit = std::mem::transmute(get(b"SDL_Quit\0")?);

        for (hint, value) in HINTS {
            let (hint, value) = (std::ffi::CString::new(hint).ok()?, std::ffi::CString::new(value).ok()?);
            set_hint(hint.as_ptr(), value.as_ptr());
        }
        if !init(INIT_GAMEPAD) {
            return None;
        }
        // Controllers SDL reads through HIDAPI show up a moment after it starts.
        for _ in 0..5 {
            pump();
            std::thread::sleep(Duration::from_millis(100));
        }
        let text = |p: *const c_char| {
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        };
        let mut count: c_int = 0;
        let ids = gamepads(&mut count);
        let mut found = Vec::new();
        if !ids.is_null() {
            for &id in std::slice::from_raw_parts(ids, usize::try_from(count).unwrap_or(0)) {
                let mapping = mapping_for(id);
                found.push((guid_text(guid_for(id).data), text(name_for(id)), inputs(&text(mapping))));
                if !mapping.is_null() {
                    free(mapping.cast());
                }
            }
            free(ids.cast());
        }
        quit();
        Some(found)
    }
}

/// The controllers in the probe's output. A controller's port counts the ones of its GUID
/// before it, as Lichii numbers them.
pub fn parse(text: &str) -> Vec<Controller> {
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            if fields.next() != Some("controller") {
                return None;
            }
            let (guid, name, mapping) = (fields.next()?, fields.next()?, fields.next()?);
            if guid.len() != 32 || mapping.is_empty() {
                return None;
            }
            let port = seen.entry(guid.to_owned()).or_insert(0);
            let controller =
                Controller { name: name.to_owned(), guid: guid.to_owned(), port: *port, mapping: mapping.to_owned() };
            *port += 1;
            Some(controller)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_controllers_of_one_kind_get_ports_0_and_1_and_another_kind_starts_at_0() {
        let xbox = "030000005e0400008e02000014010000";
        let ps5 = "050000004c050000e60c000000010000";
        let text = format!(
            "controller\t{xbox}\tXbox 360 Controller\ta:b0,b:b1\n\
             controller\t{ps5}\tPS5 Controller\ta:b0,b:b1,misc1:b15\n\
             controller\t{xbox}\tXbox 360 Controller\ta:b0,b:b1\n\
             noise\n"
        );
        let found: Vec<(String, u32)> = parse(&text).into_iter().map(|c| (c.guid, c.port)).collect();
        assert_eq!(found, [(xbox.to_owned(), 0), (ps5.to_owned(), 0), (xbox.to_owned(), 1)]);
    }

    #[test]
    fn a_guid_is_written_as_eden_writes_it_and_a_mapping_loses_its_guid_and_name() {
        let guid = [0x03, 0x00, 0xab, 0xcd, 0x5e, 0x04, 0, 0, 0x8e, 0x02, 0, 0, 0x14, 0x01, 0, 0];
        assert_eq!(guid_text(guid), "030000005e0400008e02000014010000");
        let mapping = "030000005e0400008e02000014010000,Xbox 360 Controller,a:b0,b:b1,platform:Linux,";
        assert_eq!(inputs(mapping), "a:b0,b:b1,platform:Linux");
    }
}
