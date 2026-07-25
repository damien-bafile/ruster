use std::path::PathBuf;
use ruster_tui::app::App;
use ruster_render::Renderer;
use ruster_render_raylib::RaylibRenderer;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tui = args.iter().any(|a| a == "--tui");
    let path = args.iter().skip(1).find(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .unwrap_or_default();

    // A directory argument opens ruster in dired at that location.
    let is_dir = path.is_dir();
    let content = if path.is_file() {
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

    if tui {
        let mut app = App::new(content, path.clone());
        if is_dir {
            app.open_dir(&path);
        }
        if let Err(e) = app.run_async() {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    } else {
        let mut app = App::new(content, path.clone());
        if is_dir {
            app.open_dir(&path);
        }
        let font = app.gui_font();
        let gui = app.gui_config();
        let renderer: Box<dyn Renderer> =
            Box::new(RaylibRenderer::new("ruster", gui, font.as_deref()));
        app.renderer = renderer;
        app.has_smooth_cursor = true;
        app.run_gui();
    }
}
