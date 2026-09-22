//! The header and the library must name the same functions.
//!
//! Nothing generates this header, which is the trade: no codegen step, and
//! no codegen to keep it honest either. A function added on one side and
//! forgotten on the other is a link error in an editor build, hours after
//! the change — so it is a test failure here instead.

use std::path::Path;

#[test]
fn every_exported_function_is_declared_in_the_header() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join("src/lib.rs")).unwrap();
    let header = std::fs::read_to_string(root.join("include/runity.h")).unwrap();

    let mut exported: Vec<&str> = Vec::new();
    for (index, line) in source.lines().enumerate() {
        if !line.contains("extern \"C\" fn") {
            continue;
        }
        let name = line
            .split("fn ")
            .nth(1)
            .and_then(|rest| rest.split('(').next())
            .unwrap_or_else(|| panic!("could not read a name from line {}", index + 1));
        exported.push(name);
    }
    assert!(
        exported.len() >= 14,
        "found only {} exports",
        exported.len()
    );

    for name in &exported {
        assert!(
            header.contains(name),
            "{name} is exported but not declared in include/runity.h"
        );
    }
}
