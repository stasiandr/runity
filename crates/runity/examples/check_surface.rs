//! `check_surface <file.wgsl>...` — does a material's `surface` function
//! build over the standard shader? The same check the renderer makes,
//! without a graphics card: parsed and validated, with line and column.

fn main() {
    let mut failed = false;
    for path in std::env::args().skip(1) {
        let source = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{path}: {e}");
                failed = true;
                continue;
            }
        };
        let result =
            runity::render::with_surface(runity::render::SHADER, &source).and_then(|full| {
                use wgpu::naga;
                let module =
                    naga::front::wgsl::parse_str(&full).map_err(|e| e.emit_to_string(&full))?;
                naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                .map_err(|e| e.emit_to_string(&full))?;
                Ok(())
            });
        match result {
            Ok(()) => println!("{path}: ok"),
            Err(e) => {
                println!("{path}:\n{e}");
                failed = true;
            }
        }
    }
    std::process::exit(if failed { 1 } else { 0 });
}
