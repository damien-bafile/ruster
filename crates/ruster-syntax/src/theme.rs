use ruster_render::{Color, SyntaxStyle};

fn rgb(r: u8, g: u8, b: u8) -> Color { Color::Rgb(r, g, b) }

pub fn style_for_capture(name: &str) -> SyntaxStyle {
    match name.split('.').next().unwrap_or(name) {
        "keyword"   => SyntaxStyle { fg: rgb(203, 166, 247), bg: Color::Default, bold: true,  italic: false },
        "string"    => SyntaxStyle { fg: rgb(166, 227, 161), bg: Color::Default, bold: false, italic: false },
        "comment"   => SyntaxStyle { fg: rgb(108, 112, 134), bg: Color::Default, bold: false, italic: true  },
        "function"  => SyntaxStyle { fg: rgb(137, 180, 250), bg: Color::Default, bold: false, italic: false },
        "type"      => SyntaxStyle { fg: rgb(249, 226, 175), bg: Color::Default, bold: false, italic: false },
        "variable"  => SyntaxStyle { fg: rgb(205, 214, 244), bg: Color::Default, bold: false, italic: false },
        "constant"  => SyntaxStyle { fg: rgb(250, 179, 135), bg: Color::Default, bold: false, italic: false },
        "number"    => SyntaxStyle { fg: rgb(250, 179, 135), bg: Color::Default, bold: false, italic: false },
        "operator"  => SyntaxStyle { fg: rgb(137, 220, 235), bg: Color::Default, bold: false, italic: false },
        "punctuation" => SyntaxStyle::default(),
        "builtin"   => SyntaxStyle { fg: rgb(243, 139, 168), bg: Color::Default, bold: false, italic: false },
        _           => SyntaxStyle::default(),
    }
}

/// Styles for the line-based markup highlighter (Markdown / Org). Kept beside
/// the tree-sitter theme so both share the same Catppuccin palette.
pub fn markup_style(kind: &str) -> SyntaxStyle {
    match kind {
        "heading"  => SyntaxStyle { fg: rgb(137, 180, 250), bg: Color::Default, bold: true,  italic: false },
        "strong"   => SyntaxStyle { fg: rgb(250, 179, 135), bg: Color::Default, bold: true,  italic: false },
        "emphasis" => SyntaxStyle { fg: rgb(203, 166, 247), bg: Color::Default, bold: false, italic: true  },
        "code"     => SyntaxStyle { fg: rgb(166, 227, 161), bg: Color::Default, bold: false, italic: false },
        "link"     => SyntaxStyle { fg: rgb(137, 220, 235), bg: Color::Default, bold: false, italic: false },
        "url"      => SyntaxStyle { fg: rgb(108, 112, 134), bg: Color::Default, bold: false, italic: true  },
        "marker"   => SyntaxStyle { fg: rgb(243, 139, 168), bg: Color::Default, bold: false, italic: false },
        "quote"    => SyntaxStyle { fg: rgb(108, 112, 134), bg: Color::Default, bold: false, italic: true  },
        "keyword"  => SyntaxStyle { fg: rgb(203, 166, 247), bg: Color::Default, bold: true,  italic: false },
        "block"    => SyntaxStyle { fg: rgb(166, 227, 161), bg: Color::Default, bold: false, italic: false },
        "todo"     => SyntaxStyle { fg: rgb(243, 139, 168), bg: Color::Default, bold: true,  italic: false },
        "done"     => SyntaxStyle { fg: rgb(166, 227, 161), bg: Color::Default, bold: true,  italic: false },
        _          => SyntaxStyle::default(),
    }
}

pub const RAINBOW_PALETTE: [Color; 6] = [
    Color::Rgb(243, 139, 168),  // red
    Color::Rgb(250, 179, 135),  // peach
    Color::Rgb(249, 226, 175),  // yellow
    Color::Rgb(166, 227, 161),  // green
    Color::Rgb(137, 190, 180),  // teal
    Color::Rgb(137, 180, 250),  // blue
];
