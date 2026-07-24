use std::collections::HashMap;
use std::path::Path;

pub fn parse(file_path: &Path) -> HashMap<String, String> {
    let mut result = HashMap::new();
    let mut dir = file_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    loop {
        let ec_path = dir.join(".editorconfig");
        if ec_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&ec_path) {
                if let Some(section) = match_glob(&content, file_path) {
                    for (k, v) in section {
                        result.insert(k.to_string(), v.to_string());
                    }
                }
                if has_root_marker(&content) {
                    break;
                }
            }
        }
        if !dir.pop() {
            break;
        }
    }
    result
}

fn has_root_marker(content: &str) -> bool {
    content.lines().any(|l| l.trim().eq_ignore_ascii_case("root = true"))
}

fn match_glob<'a>(content: &'a str, file_path: &Path) -> Option<HashMap<&'a str, &'a str>> {
    let file_name = file_path.file_name()?.to_str()?;
    let mut best: Option<(usize, HashMap<&'a str, &'a str>)> = None;
    let mut lines = content.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with(';') && !l.starts_with('#'));
    while let Some(line) = lines.next() {
        if line.starts_with('[') {
            if let Some(end) = line.find(']') {
                let pattern = &line[1..end];
                if matches_glob_pattern(pattern, file_name) || matches_glob_pattern(pattern, file_path.to_str().unwrap_or("")) {
                    let mut props = HashMap::new();
                    for val_line in lines.by_ref() {
                        if val_line.starts_with('[') {
                            break; // next section; peeked
                        }
                        if let Some(eq) = val_line.find('=') {
                            let key = val_line[..eq].trim();
                            let val = val_line[eq+1..].trim();
                            props.insert(key, val);
                        }
                    }
                    let specificity = pattern.len();
                    if best.as_ref().is_none_or(|(s, _)| specificity > *s) {
                        best = Some((specificity, props));
                    }
                }
            }
        }
    }
    best.map(|(_, m)| m)
}

fn matches_glob_pattern(pattern: &str, name: &str) -> bool {
    // Simple glob matching: * matches anything except /, ** matches everything
    // ?, [seq], {a,b} are not implemented yet
    if pattern == "*" {
        return true;
    }
    if pattern == "**" || pattern == "**/" {
        return true;
    }
    // Single * at start and end
    if pattern.starts_with("*") && pattern.ends_with("*") {
        let inner = &pattern[1..pattern.len()-1];
        return name.contains(inner);
    }
    if let Some(suffix) = pattern.strip_prefix("*") {
        return name.ends_with(suffix);
    }
    if let Some(prefix) = pattern.strip_suffix("*") {
        return name.starts_with(prefix);
    }
    pattern == name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_star() {
        assert!(matches_glob_pattern("*", "foo.rs"));
    }

    #[test]
    fn matches_extension() {
        assert!(matches_glob_pattern("*.rs", "main.rs"));
        assert!(!matches_glob_pattern("*.rs", "main.py"));
    }

    #[test]
    fn no_dot_editorconfig_returns_empty() {
        let tmp = std::env::temp_dir();
        let result = parse(&tmp.join("nonexistent.txt"));
        assert!(result.is_empty());
    }

    #[test]
    fn parse_with_dot_editorconfig() {
        use std::io::Write;
        let tmp = std::env::temp_dir().join("ruster_ec_test");
        let _ = std::fs::create_dir_all(&tmp);
        let mut f = std::fs::File::create(tmp.join(".editorconfig")).unwrap();
        write!(f, "root = true\n\n[*]\nindent_style = space\nindent_size = 2\n").unwrap();
        let file = tmp.join("test.rs");
        std::fs::File::create(&file).unwrap();
        let props = parse(&file);
        assert_eq!(props.get("indent_style").map(|s| s.as_str()), Some("space"));
        assert_eq!(props.get("indent_size").map(|s| s.as_str()), Some("2"));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
