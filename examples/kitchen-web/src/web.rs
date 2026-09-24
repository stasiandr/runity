//! The page around the game, in the browser (`web/index.html`). Nothing
//! here off the web: on the desktop every function is empty.
//!
//! * The project's data — scenes, prefabs, screens, strings, the built
//!   library — was fetched by the page as one file (`data.bin`, made by
//!   `web/pack.py`) before the game started; [`start`] puts it where the
//!   game looks for it (`runity::files::mount`), so the rest of the game
//!   reads its files as it does on a disk.
//! * What the game writes — the player's prefs, the best score — goes to
//!   the browser's storage and is put back on the next visit.
//! * The page shows its sticks and buttons only in the kitchen: [`phase`]
//!   tells it where the game is.

#[cfg(target_arch = "wasm32")]
mod page {
    use wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen]
    extern "C" {
        /// The project, as the page fetched it.
        #[wasm_bindgen(js_namespace = runityPage, js_name = data)]
        pub fn data() -> js_sys::Uint8Array;
        /// The player's files from the last visit, packed as `data` is.
        #[wasm_bindgen(js_namespace = runityPage, js_name = saved)]
        pub fn saved() -> js_sys::Uint8Array;
        /// Keep a file the game wrote.
        #[wasm_bindgen(js_namespace = runityPage, js_name = save)]
        pub fn save(path: &str, bytes: &[u8]);
        /// Where the game is: `menu`, `joining`, `lobby`, `kitchen`, `paused`.
        #[wasm_bindgen(js_namespace = runityPage, js_name = phase)]
        pub fn phase(name: &str);
        /// The quality the page asks for: `?quality=` in its address, or
        /// `low` on a phone; empty to leave it to the device.
        #[wasm_bindgen(js_namespace = runityPage, js_name = quality)]
        pub fn quality() -> String;
    }
}

/// Files packed one after another: a count, then each file's path and
/// bytes, every length a little-endian `u32`.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn unpack(mut bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut take = |n: usize| -> Option<&[u8]> {
        let (head, rest) = (bytes.get(..n)?, bytes.get(n..)?);
        bytes = rest;
        Some(head)
    };
    let number = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
    let mut out = Vec::new();
    let Some(count) = take(4).map(number) else {
        return out;
    };
    for _ in 0..count {
        let Some(n) = take(4).map(number) else { break };
        let Some(path) = take(n).map(|p| String::from_utf8_lossy(p).into_owned()) else {
            break;
        };
        let Some(n) = take(4).map(number) else { break };
        let Some(body) = take(n).map(<[u8]>::to_vec) else {
            break;
        };
        out.push((path, body));
    }
    out
}

/// The page's data under `root`, its randomness in fresh ids, a panic
/// said in the console.
pub fn start(root: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        runity::files::mount(root, unpack(&page::data().to_vec()));
        runity::files::mount("/", unpack(&page::saved().to_vec()));
        runity::files::on_write(|path, bytes| page::save(&path.to_string_lossy(), bytes));
        let random = || (js_sys::Math::random() * u32::MAX as f64) as u64;
        runity::EntityId::seed(random() << 32 | random());
    }
    let _ = root;
}

/// Tell the page where the game is, when that changed.
pub fn phase(name: &'static str) {
    #[cfg(target_arch = "wasm32")]
    {
        use std::sync::Mutex;
        static LAST: Mutex<&str> = Mutex::new("");
        let mut last = LAST.lock().unwrap();
        if *last != name {
            *last = name;
            page::phase(name);
        }
    }
    let _ = name;
}

/// The quality the page asks for, if it asks.
pub fn quality() -> Option<runity::quality::Quality> {
    #[cfg(target_arch = "wasm32")]
    {
        use runity::quality::Quality;
        return match page::quality().to_ascii_lowercase().as_str() {
            "low" => Some(Quality::Low),
            "medium" => Some(Quality::Medium),
            "high" => Some(Quality::High),
            "ultra" => Some(Quality::Ultra),
            _ => None,
        };
    }
    #[allow(unreachable_code)]
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn packed_files_come_back_as_they_went_in() {
        let mut packed = 2u32.to_le_bytes().to_vec();
        for (path, body) in [
            ("ui/menu.ron", &b"()"[..]),
            ("library/a.rasset", &[1, 2, 3][..]),
        ] {
            packed.extend((path.len() as u32).to_le_bytes());
            packed.extend(path.as_bytes());
            packed.extend((body.len() as u32).to_le_bytes());
            packed.extend(body);
        }
        let files = super::unpack(&packed);
        assert_eq!(files.len(), 2);
        assert_eq!(files[1], ("library/a.rasset".to_string(), vec![1, 2, 3]));
        assert!(
            super::unpack(&packed[..9]).is_empty(),
            "a cut file is left out"
        );
    }
}
