//! Package-relative paths (`$defs/relativePath`, architecture section 8).

/// Longest path in a manifest or package.
pub const MAX_PATH_LEN: usize = 128;

/// Checks a path relative to the package root. The error completes a
/// sentence that starts with the quoted path.
pub fn check_relative(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("is empty");
    }
    if path.len() > MAX_PATH_LEN {
        return Err("is longer than 128 characters");
    }
    let bytes = path.as_bytes();
    if path.starts_with('/') || path.starts_with('\\') || (bytes.len() > 1 && bytes[1] == b':') {
        return Err("is absolute; paths are relative to the package root");
    }
    if path.contains('\\') {
        return Err("uses a backslash; use / as the separator");
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.iter().any(|s| *s == "." || *s == "..") {
        return Err("contains a . or .. segment, which could leave the package");
    }
    if segments.iter().any(|s| s.is_empty()) {
        return Err("has an empty segment (// or a trailing /)");
    }
    let valid = segments.iter().all(|segment| {
        let mut chars = segment.chars();
        chars
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    });
    if !valid {
        return Err(
            "may only use letters, digits, _, . and -, and no segment may start with . or -",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_relative;

    #[test]
    fn relative_paths() {
        for good in ["src/index.ts", "assets/keys/pr.png", "a", "_x/y.z-1"] {
            assert_eq!(check_relative(good), Ok(()), "{good}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "\\server\\share",
            "C:/x",
            "src\\index.ts",
            "../index.ts",
            "src/../../x",
            "./src/index.ts",
            "src//index.ts",
            "src/",
            ".hidden",
            "src/-x",
            "src/in dex.ts",
            "src/índex.ts",
        ] {
            assert!(check_relative(bad).is_err(), "{bad}");
        }
        assert!(check_relative(&"a".repeat(129)).is_err());
    }
}
