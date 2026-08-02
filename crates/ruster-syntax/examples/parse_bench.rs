fn main() {
    let path = std::env::args().nth(1).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let lines = text.lines().count();
    println!("{path}: {lines} lines, {} KB", text.len() / 1024);

    let buf = ruster_core::buffer::Buffer::from_str(&text);
    let t = std::time::Instant::now();
    for _ in 0..20 { std::hint::black_box(buf.to_string()); }
    println!("  rope to_string   {:>7.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);

    let mut e = ruster_syntax::SyntaxEngine::new(&text, "rs").unwrap();
    let t = std::time::Instant::now();
    for _ in 0..20 { e.reparse(&text); }
    println!("  reparse          {:>7.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);

    let kws = vec!["TODO".to_string(), "FIXME".to_string()];
    let style = ruster_render::SyntaxStyle::default();
    let t = std::time::Instant::now();
    for _ in 0..20 { e.overlay_todo_highlights(&kws, style); }
    println!("  todo overlay     {:>7.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
}
