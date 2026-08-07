//! Load a runtime grammar and actually use it — the check the unit tests
//! cannot make.
//!
//! `grammar.rs`'s tests cover the pure ABI predicate and the symbol lookup, but
//! neither loads a real grammar, because building one needs a C compiler and a
//! tree-sitter CLI that CI does not have. The `unsafe` path — `dlopen`, `dlsym`,
//! call the entry point, read the ABI, then hand the language to a parser and a
//! query — is exactly where a mistake segfaults instead of erroring, so it is
//! worth being able to exercise on demand.
//!
//! Build a grammar from any `tree-sitter-*` crate already in the cargo registry:
//!
//! ```text
//! SRC=$(ls -d ~/.cargo/registry/src/*/tree-sitter-json-0.24*/src | head -1)
//! mkdir -p /tmp/g/ruster/grammars
//! cc -shared -fPIC -O1 -I "$SRC" "$SRC/parser.c" \
//!    -o /tmp/g/ruster/grammars/libtree-sitter-json.dylib
//! cargo run -p ruster-syntax --example load_runtime_grammar -- /tmp/g/ruster/grammars
//! ```
//!
//! To exercise the ABI gate, copy `$SRC` somewhere writable and edit
//! `#define LANGUAGE_VERSION` in `parser.c` before compiling — `-D` will not do
//! it, the file defines the macro itself.
//!
//! Verified on 2026-08-01 against real libraries: ABI 12 refused, 13 and 15
//! accepted, 99 refused. With a refused grammar in place the editor warns, falls
//! back to the built-in and keeps highlighting, rather than crashing.

use streaming_iterator::StreamingIterator;

fn main() {
    let Some(arg) = std::env::args().nth(1) else {
        eprintln!("usage: load_runtime_grammar <dir containing libtree-sitter-json.*>");
        std::process::exit(2);
    };
    let dir = std::path::PathBuf::from(arg);

    let lang = match ruster_syntax::grammar::load_grammar(&dir, "json") {
        Ok(lang) => {
            println!(
                "loaded: abi={} node_kinds={}",
                lang.abi_version(),
                lang.node_kind_count()
            );
            lang
        }
        Err(e) => {
            println!("refused: {e}");
            return;
        }
    };

    // Loading is the easy half. Parsing and querying are what dereference the
    // language's function tables, so a wrong ABI fails here rather than above.
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&lang).expect("set_language");
    let src = r#"{"a": [1, 2], "b": "x"}"#;
    let tree = parser.parse(src, None).expect("parse");
    println!("root: {}", tree.root_node().to_sexp());

    let query = tree_sitter::Query::new(&lang, "(string) @s (number) @n").expect("query");
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), src.as_bytes());
    let mut n = 0;
    while matches.next().is_some() {
        n += 1;
    }
    println!("query matches: {n}");
}
