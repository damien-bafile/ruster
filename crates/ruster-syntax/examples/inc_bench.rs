// Does incremental parsing actually pay for the plumbing it needs?
use tree_sitter::{InputEdit, Parser, Point};

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let lang = ruster_syntax::language_for_ext("rs").unwrap();
    let mut p = Parser::new();
    p.set_language(&lang).unwrap();
    println!("{}: {} lines", path, text.lines().count());

    let t = std::time::Instant::now();
    let mut tree = p.parse(&text, None).unwrap();
    println!(
        "  cold parse        {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );

    // A one-character insert in the middle, the common case while typing.
    let at = text.len() / 2;
    let at = (at..text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap();
    let row = text[..at].matches('\n').count();
    let col = at - text[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let mut edited = text.clone();
    edited.insert(at, 'x');

    let t = std::time::Instant::now();
    for _ in 0..20 {
        let mut tr = tree.clone();
        tr.edit(&InputEdit {
            start_byte: at,
            old_end_byte: at,
            new_end_byte: at + 1,
            start_position: Point::new(row, col),
            old_end_position: Point::new(row, col),
            new_end_position: Point::new(row, col + 1),
        });
        std::hint::black_box(p.parse(&edited, Some(&tr)).unwrap());
    }
    println!(
        "  incremental       {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 20.0
    );

    let t = std::time::Instant::now();
    for _ in 0..5 {
        std::hint::black_box(p.parse(&edited, None).unwrap());
    }
    println!(
        "  full (no tree)    {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 5.0
    );
    tree = p.parse(&edited, None).unwrap();

    // `reparse` costs far more than the parse alone. What is the rest?
    let depths = ruster_syntax::bench_bracket_depths(&edited);
    let t = std::time::Instant::now();
    for _ in 0..5 {
        std::hint::black_box(ruster_syntax::bench_bracket_depths(&edited));
    }
    println!(
        "  bracket depths    {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 5.0
    );

    let mut h = ruster_syntax::highlighter::Highlighter::new(
        lang.clone(),
        ruster_syntax::bench_builtin_query("rust"),
        "rust",
    )
    .unwrap();
    let t = std::time::Instant::now();
    for _ in 0..5 {
        std::hint::black_box(h.highlight_lines(&tree, &edited, &depths, None));
    }
    println!(
        "  highlight_lines   {:>7.2} ms",
        t.elapsed().as_secs_f64() * 1000.0 / 5.0
    );
}
