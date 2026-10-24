//! Embeds the console catalog files in the project's `assets/catalog/` into the binary, by
//! file name. Only the names an emulator can ask for are bundled; other files there are not.
//! Also embeds the catalog sources, the project's `assets/catalog-sources.json`.

use std::path::Path;

/// The list `BUNDLED_NAMES` in `src/lib.rs` reads too.
const NAMES: [&str; 3] = include!("bundled-names.rs");

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/catalog");
    let entries: String = NAMES
        .iter()
        .map(|n| {
            let file = root.join(n);
            assert!(file.is_file(), "the project's assets/catalog/{n}");
            println!("cargo:rerun-if-changed={}", file.display());
            format!(
                "    ({n:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../../assets/catalog/{n}\"))),\n"
            )
        })
        .collect();
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(out.join("bundled.rs"), format!("&[\n{entries}]")).expect("write bundled.rs");

    let sources = root.join("../catalog-sources.json");
    assert!(sources.is_file(), "the project's assets/catalog-sources.json");
    println!("cargo:rerun-if-changed={}", sources.display());
    let include = "include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../../assets/catalog-sources.json\"))";
    std::fs::write(out.join("sources.rs"), include).expect("write sources.rs");
}
