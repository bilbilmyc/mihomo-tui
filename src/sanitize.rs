/// Bound and sanitize untrusted text before it reaches a terminal or report.
pub fn text(value: &str, secret: Option<&str>) -> String {
    let value: String = value
        .chars()
        .take(8192)
        .map(|character| {
            if character.is_control()
                || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                ' '
            } else {
                character
            }
        })
        .collect();
    let value = match secret.filter(|value| !value.is_empty()) {
        Some(secret) => value.replace(secret, "<密钥已隐藏>"),
        None => value,
    };
    let mut result = String::new();
    for word in value.split_whitespace() {
        if !result.is_empty() {
            result.push(' ');
        }
        if let Some(index) = word.find("://") {
            let start = word[..index]
                .char_indices()
                .rev()
                .find(|(_, character)| !character.is_ascii_alphanumeric() && *character != '+')
                .map_or(0, |(index, character)| index + character.len_utf8());
            result.push_str(&word[..start]);
            result.push_str("<URL已隐藏>");
        } else {
            result.extend(word.chars().filter(|character| {
                !character.is_control()
                    && !matches!(*character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            }));
        }
    }
    if result.chars().count() > 2048 {
        result = result.chars().take(2048).collect();
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn hides_urls_secrets_and_terminal_controls() {
        let text = super::text(
            "failure https://user:pass@sub.example/private?token=hidden secret-value\u{1b}[2J\nnext",
            Some("secret-value"),
        );
        assert!(!text.contains("pass"));
        assert!(!text.contains("hidden"));
        assert!(!text.contains("secret-value"));
        assert!(!text.contains('\u{1b}'));
        assert!(!text.contains('\n'));
    }

    #[test]
    fn bounds_untrusted_display_text_without_breaking_unicode() {
        assert!(super::text(&"中".repeat(5000), None).chars().count() <= 2049);
    }

    #[test]
    fn url_prefixes_cannot_preserve_escape_codes_or_split_unicode() {
        assert!(!super::text("\u{001b}https://example.com/private", None).contains('\u{001b}'));
        assert!(!super::text("订阅https://example.com/private", None).contains("private"));
    }
}
