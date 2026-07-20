use std::path::PathBuf;
use ruster_tui::app::App;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = match args.len() {
        2 => PathBuf::from(&args[1]),
        _ => {
            eprintln!("Usage: ruster <file>");
            std::process::exit(1);
        }
    };

    let content = if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error reading {}: {}", path.display(), e);
                std::process::exit(1);
            }
        }
    } else {
        String::new()
    };

    let mut app = App::new(content, path);
    if let Err(e) = app.run_async() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
