use std::path::Path;

fn main() {
    let Some(bundle) = std::env::args_os().nth(1) else {
        eprintln!("A VST3 bundle path is required.");
        std::process::exit(2);
    };
    std::process::exit(rustel_runtime::vst::serve(Path::new(&bundle)));
}
