//! Identifier helpers shared by generators.

/// `snake_case` / `kebab-case` / `PascalCase` -> `camelCase`.
pub fn camel(s: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for (i, c) in s.chars().enumerate() {
        if c == '_' || c == '-' || c == ' ' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else if i == 0 {
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Escape a doc comment line so it cannot close the comment.
pub fn doc_line(s: &str) -> String {
    s.replace("*/", "* /")
}
