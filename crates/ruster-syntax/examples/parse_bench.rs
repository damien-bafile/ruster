fn main() {
    let path = std::env::args().nth(1).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let lines = text.lines().count();
    println!("{path}: {lines} lines, {} KB", text.len() / 1024);

    let buf = ruster_core::buffer::Buffer::from_str(&text);
    let t = std::time::Instant::now();
    for _ in 0..20 {
        std::hint::black_box(buf.to_string());
    }
    println!(
        "  rope to_string   {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 20.0
    );

    let mut e = ruster_syntax::SyntaxEngine::new(&text, "rs").unwrap();
    let t = std::time::Instant::now();
    for _ in 0..20 {
        e.reparse(&text);
    }
    println!(
        "  reparse          {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 20.0
    );

    // The path the editor actually takes while typing: one character in, with
    // the edit recorded, versus the full reparse it used to do.
    let mut b = ruster_core::buffer::Buffer::from_str(&text);
    let at = text.chars().count() / 2;
    let mut inc = ruster_syntax::SyntaxEngine::new(&text, "rs").unwrap();
    let t = std::time::Instant::now();
    for i in 0..20 {
        b.insert(at + i, "x");
        let s = b.to_string();
        inc.reparse_with_edits(&s, &b.take_edits());
    }
    println!(
        "  reparse (edit)   {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 20.0
    );

    // The same edit with the highlight bounded to a screenful, which is what
    // the editor does — a window is about 50 lines tall.
    let mut b = ruster_core::buffer::Buffer::from_str(&text);
    let mut vp = ruster_syntax::SyntaxEngine::new(&text, "rs").unwrap();
    let at_line = text.lines().count() / 2;
    vp.set_viewport(at_line, at_line + 50);
    let t = std::time::Instant::now();
    for i in 0..20 {
        b.insert(at + i, "x");
        let s = b.to_string();
        vp.reparse_with_edits(&s, &b.take_edits());
    }
    println!(
        "  reparse (viewport) {:>5.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 20.0
    );

    // Scrolling far enough to leave the margin: the worst case a user can
    // provoke by holding a movement key.
    let t = std::time::Instant::now();
    for i in 0..20 {
        vp.set_viewport(i * 500, i * 500 + 50);
    }
    println!(
        "  scroll past margin {:>5.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 20.0
    );

    // What `render` pays every frame: cloning every styled line into the
    // frame state, even though the widget draws only what fits on screen.
    let all = e.styled_lines().to_vec();
    let t = std::time::Instant::now();
    for _ in 0..20 {
        std::hint::black_box(e.styled_lines().to_vec());
    }
    println!(
        "  clone all lines  {:>7.2} ms  ({} lines)",
        t.elapsed().as_secs_f64() * 1000.0 / 20.0,
        all.len()
    );

    let t = std::time::Instant::now();
    for _ in 0..20 {
        std::hint::black_box(e.styled_lines()[..50.min(all.len())].to_vec());
    }
    println!(
        "  clone 50 lines   {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 20.0
    );

    let kws = vec!["TODO".to_string(), "FIXME".to_string()];
    let style = ruster_render::SyntaxStyle::default();
    let t = std::time::Instant::now();
    for _ in 0..20 {
        e.overlay_todo_highlights(&kws, style);
    }
    println!(
        "  todo overlay     {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 20.0
    );
}
