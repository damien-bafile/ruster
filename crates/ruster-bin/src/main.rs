use std::path::PathBuf;
use ruster_tui::app::App;
use ruster_render::Renderer;
use ruster_render_raylib::RaylibRenderer;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let gui = args.iter().any(|a| a == "--gui");
    let path = match args.iter().skip(1).find(|a| !a.starts_with('-')) {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("Usage: ruster [--gui] <file>");
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

    if gui {
        let renderer: Box<dyn Renderer> = Box::new(RaylibRenderer::new(800, 600, "ruster"));
        let mut app = App::new(content, path);
        app.renderer = renderer;
        app.run_gui();
    } else {
        let mut app = App::new(content, path);
        if let Err(e) = app.run_async() {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}
