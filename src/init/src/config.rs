use std::io::BufRead;

/// Parse `KEY=VALUE` lines from `reader`.
///
/// Lines starting with `#` and blank lines are skipped.
/// Values may optionally be double-quoted; quotes are stripped but no
/// escape processing is done (the parent sends plain env-file content).
pub fn read_config<R: BufRead>(reader: R) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim().to_string();
            let val = v.trim().trim_matches('"').to_string();
            if !key.is_empty() {
                out.push((key, val));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;

    #[test]
    fn parses_basic_pairs() {
        let input = b"DATABASE_URL=postgresql://user@host/db\nREDIS_URL=redis://localhost\n";
        let pairs = read_config(BufReader::new(input.as_ref()));
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0], ("DATABASE_URL".into(), "postgresql://user@host/db".into()));
        assert_eq!(pairs[1], ("REDIS_URL".into(), "redis://localhost".into()));
    }

    #[test]
    fn skips_comments_and_blanks() {
        let input = b"# comment\n\nKEY=val\n\n# another\n";
        let pairs = read_config(BufReader::new(input.as_ref()));
        assert_eq!(pairs, vec![("KEY".into(), "val".into())]);
    }

    #[test]
    fn strips_double_quotes() {
        let input = b"SECRET=\"my secret value\"\n";
        let pairs = read_config(BufReader::new(input.as_ref()));
        assert_eq!(pairs, vec![("SECRET".into(), "my secret value".into())]);
    }

    #[test]
    fn value_may_contain_equals() {
        let input = b"URL=postgres://user:pass@host/db?sslmode=require\n";
        let pairs = read_config(BufReader::new(input.as_ref()));
        assert_eq!(pairs[0].1, "postgres://user:pass@host/db?sslmode=require");
    }

    #[test]
    fn empty_input_returns_empty() {
        let pairs = read_config(BufReader::new(b"".as_ref()));
        assert!(pairs.is_empty());
    }
}
