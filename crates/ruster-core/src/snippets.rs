//! A minimal LuaSnip-style snippet engine: parse snippet bodies with tabstops
//! (`$1`, `$2`, `$0`, `${1:default}`) into inserted text plus ordered tabstop
//! positions.

use std::collections::HashMap;

/// A tabstop in expanded text: its `$N` index and char range (start..end).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabstop {
    pub index: u32,
    pub start: usize,
    pub end: usize,
}

/// The result of expanding a snippet body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    /// The literal text to insert (with `${N:default}` defaults filled in).
    pub text: String,
    /// Tabstops in visiting order: `$1`, `$2`, … then `$0` last.
    pub stops: Vec<Tabstop>,
}

/// Parse a snippet body into inserted text + ordered tabstops.
pub fn expand(body: &str) -> Expansion {
    let chars: Vec<char> = body.chars().collect();
    let mut text = String::new();
    let mut stops: Vec<Tabstop> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // Escaped dollar.
        if c == '\\' && i + 1 < chars.len() && chars[i + 1] == '$' {
            text.push('$');
            i += 2;
            continue;
        }
        if c == '$' && i + 1 < chars.len() {
            // ${N:default}
            if chars[i + 1] == '{' {
                let mut j = i + 2;
                let mut num = String::new();
                while j < chars.len() && chars[j].is_ascii_digit() {
                    num.push(chars[j]);
                    j += 1;
                }
                let mut default = String::new();
                if j < chars.len() && chars[j] == ':' {
                    j += 1;
                    while j < chars.len() && chars[j] != '}' {
                        default.push(chars[j]);
                        j += 1;
                    }
                }
                if j < chars.len() && chars[j] == '}' {
                    j += 1;
                }
                if let Ok(index) = num.parse::<u32>() {
                    let start = text.chars().count();
                    text.push_str(&default);
                    let end = text.chars().count();
                    stops.push(Tabstop { index, start, end });
                    i = j;
                    continue;
                }
            }
            // $N
            if chars[i + 1].is_ascii_digit() {
                let mut j = i + 1;
                let mut num = String::new();
                while j < chars.len() && chars[j].is_ascii_digit() {
                    num.push(chars[j]);
                    j += 1;
                }
                if let Ok(index) = num.parse::<u32>() {
                    let pos = text.chars().count();
                    stops.push(Tabstop { index, start: pos, end: pos });
                    i = j;
                    continue;
                }
            }
        }
        text.push(c);
        i += 1;
    }
    // Visit order: 1, 2, 3, … then 0 (final cursor) last.
    stops.sort_by_key(|s| if s.index == 0 { u32::MAX } else { s.index });
    Expansion { text, stops }
}

/// Snippet definitions keyed by filetype then trigger word.
#[derive(Default)]
pub struct SnippetSet {
    map: HashMap<String, HashMap<String, String>>,
}

impl SnippetSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// A small built-in set so snippets work without any config.
    pub fn builtin() -> Self {
        let mut s = Self::new();
        s.insert("rust", "fn", "fn ${1:name}(${2:args}) {\n    $0\n}");
        s.insert("rust", "pfn", "pub fn ${1:name}(${2:args}) {\n    $0\n}");
        s.insert("rust", "impl", "impl ${1:Type} {\n    $0\n}");
        s.insert("rust", "test", "#[test]\nfn ${1:name}() {\n    $0\n}");
        s.insert("python", "def", "def ${1:name}(${2:args}):\n    $0");
        s.insert("python", "class", "class ${1:Name}:\n    $0");
        s.insert("lua", "fn", "function ${1:name}(${2:args})\n    $0\nend");
        s
    }

    pub fn insert(&mut self, filetype: &str, trigger: &str, body: &str) {
        self.map
            .entry(filetype.to_string())
            .or_default()
            .insert(trigger.to_string(), body.to_string());
    }

    pub fn get(&self, filetype: &str, trigger: &str) -> Option<&str> {
        self.map.get(filetype)?.get(trigger).map(|s| s.as_str())
    }

    /// Load `<filetype>.snippets` files from `dir`. Each non-empty, non-comment
    /// line is `trigger<TAB>body`, where `\n` in the body is a literal newline.
    pub fn load_dir(&mut self, dir: &std::path::Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let filetype = match path.file_stem().and_then(|s| s.to_str()) {
                Some(f) => f.to_string(),
                None => continue,
            };
            if path.extension().and_then(|e| e.to_str()) != Some("snippets") {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            for line in text.lines() {
                let line = line.trim_end();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((trigger, body)) = line.split_once('\t') {
                    let body = body.replace("\\n", "\n").replace("\\t", "\t");
                    self.insert(&filetype, trigger.trim(), &body);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tabstops_and_defaults() {
        let e = expand("fn ${1:name}() {\n    $0\n}");
        assert_eq!(e.text, "fn name() {\n    \n}");
        // stop 1 covers "name", stop 0 is the body position, visited last.
        assert_eq!(e.stops.len(), 2);
        assert_eq!(e.stops[0].index, 1);
        assert_eq!(&e.text.chars().collect::<String>()[e.stops[0].start..e.stops[0].end], "name");
        assert_eq!(e.stops[1].index, 0);
    }

    #[test]
    fn plain_tabstops_are_zero_width_and_ordered() {
        let e = expand("$2 $1 $0");
        assert_eq!(e.text, "  ");
        // visiting order: 1, 2, 0
        let order: Vec<u32> = e.stops.iter().map(|s| s.index).collect();
        assert_eq!(order, vec![1, 2, 0]);
    }

    #[test]
    fn escaped_dollar_is_literal() {
        let e = expand("cost is \\$5");
        assert_eq!(e.text, "cost is $5");
        assert!(e.stops.is_empty());
    }

    #[test]
    fn builtin_lookup() {
        let s = SnippetSet::builtin();
        assert!(s.get("rust", "fn").is_some());
        assert!(s.get("rust", "nope").is_none());
        assert!(s.get("python", "def").unwrap().contains("def"));
    }
}
