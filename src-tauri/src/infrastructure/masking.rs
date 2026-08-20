//! Secret masking for process output (S3/S8).
//!
//! Anything leaving a child process is passed through `mask_secrets` before
//! it can reach logs, errors, or the UI. Per S3 this covers API keys,
//! tokens, passwords, and channel credentials — not just `sk-` keys.

/// Mask used for redacted secret values.
const MASK: &str = "****";

/// `sk-`-prefixed API keys (e.g. OpenAI-style). The token part must be at
/// least 8 characters of `[A-Za-z0-9_-]`; shorter `sk-` prefixes are left.
const SK_PREFIX: &str = "sk-";

/// Substrings whose occurrence inside an identifier marks a candidate
/// secret key name (checked case-insensitively).
const SECRET_KEYWORDS: [&str; 7] = [
    "token",
    "password",
    "passwd",
    "key",
    "credential",
    "secret",
    "apikey",
];

/// Last key segments (lowercase, split on `_`/`-`/`.`) that are always
/// secret-bearing, e.g. `auth_token`, `gateway.auth.token`.
const SECRET_KEY_SEGMENTS: [&str; 7] = [
    "token",
    "password",
    "passwd",
    "credential",
    "credentials",
    "secret",
    "apikey",
];

/// Normalized key names (separators removed, lowercased) that are always
/// secret-bearing, e.g. `api_key` / `api-key` / `apiKey`.
const SECRET_KEY_NORMS: [&str; 1] = ["apikey"];

/// Normalized key suffixes that mark compound (e.g. camelCase) secret key
/// names, e.g. `authToken`, `clientSecret`, `X-Api-Key`.
const SECRET_KEY_SUFFIXES: [&str; 6] = [
    "token",
    "password",
    "passwd",
    "credential",
    "secret",
    "apikey",
];

/// Masks all secret material (S3) in `input`:
///
/// - `sk-<token>` API keys become `sk-****`
/// - secret key/value pairs (`token=...`, `"password": "..."`,
///   `OPENCLAW_GATEWAY_TOKEN=...`, `APIKey: ...`) have their value masked
/// - `Bearer <token>` auth values become `Bearer ****`
///
/// Values that cannot be secrets (`null`, `true`, `false`, numbers, empty)
/// are left untouched so structured payloads stay parseable.
pub fn mask_secrets(input: &str) -> String {
    let masked = mask_sk_secrets(input);
    let masked = mask_keyed_secrets(&masked);
    mask_bearer_tokens(&masked)
}

/// First pass: mask `sk-`-prefixed API keys.
fn mask_sk_secrets(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(pos) = rest.find(SK_PREFIX) {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + SK_PREFIX.len()..];
        let secret_len = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .count();
        if secret_len >= 8 {
            out.push_str("sk-****");
            // All matched characters are ASCII, so byte offset == char count.
            rest = &after[secret_len..];
        } else {
            out.push_str(SK_PREFIX);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Second pass: mask the value of secret key/value pairs.
fn mask_keyed_secrets(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let lower: Vec<char> = input.chars().map(|c| c.to_ascii_lowercase()).collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    while i < chars.len() {
        let Some((start, kw_len)) = next_secret_keyword(&lower, i) else {
            out.extend(chars[i..].iter());
            break;
        };
        // Expand the keyword to the full key identifier around it.
        let key_start = expand_key_backward(&chars, start);
        let key_end = expand_key_forward(&chars, start + kw_len);
        if key_start < i || !is_secret_key(&chars[key_start..key_end]) {
            out.extend(chars[i..start + kw_len].iter());
            i = start + kw_len;
            continue;
        }
        // Separator: optional whitespace, optional closing quote, whitespace,
        // then `:` or `=` (covers `token=`, `token:`, and JSON `"token":`).
        let mut j = skip_spaces(&chars, key_end);
        if matches!(chars.get(j), Some(&'"') | Some(&'\'')) {
            j += 1;
            j = skip_spaces(&chars, j);
        }
        if !matches!(chars.get(j), Some(&':') | Some(&'=')) {
            // A secret-named word without a value is not a leak.
            out.extend(chars[i..key_end].iter());
            i = key_end;
            continue;
        }
        let value_start = skip_spaces(&chars, j + 1);
        let value_end = value_end(&chars, value_start);
        if value_end <= value_start || is_inert_value(&chars[value_start..value_end]) {
            out.extend(chars[i..value_end].iter());
            i = value_end;
            continue;
        }
        let quoted = matches!(chars.get(value_start), Some(&'"') | Some(&'\''));
        out.extend(chars[i..value_start].iter());
        if quoted {
            // Keep the surrounding quotes; redact only the value inside.
            let quote = chars[value_start];
            out.push(quote);
            out.push_str(MASK);
            out.push(quote);
        } else {
            out.push_str(MASK);
        }
        i = value_end;
    }
    out
}

/// Third pass: mask `Bearer <token>` authorization values.
fn mask_bearer_tokens(input: &str) -> String {
    const WORD: &str = "bearer";
    let lower = input.to_ascii_lowercase();
    let mut rest_in = input;
    let mut rest_low = lower.as_str();
    let mut out = String::with_capacity(input.len());
    while let Some(pos) = rest_low.find(WORD) {
        let whole_word = pos
            .checked_sub(1)
            .is_none_or(|before| !rest_low.as_bytes()[before].is_ascii_alphanumeric())
            && rest_low.as_bytes()[pos + WORD.len()..]
                .first()
                .is_none_or(|next| !next.is_ascii_alphanumeric());
        if !whole_word {
            out.push_str(&rest_in[..pos + WORD.len()]);
            rest_in = &rest_in[pos + WORD.len()..];
            rest_low = &rest_low[pos + WORD.len()..];
            continue;
        }
        let tail_low = &rest_low[pos + WORD.len()..];
        let mut ws = 0usize;
        while ws < tail_low.len() && tail_low.as_bytes()[ws].is_ascii_whitespace() {
            ws += 1;
        }
        let value = &tail_low[ws..];
        let value_len = value
            .find(|c: char| {
                c.is_whitespace() || matches!(c, '"' | '\'' | ',' | '}' | ']' | ')' | ';')
            })
            .unwrap_or(value.len());
        if value_len == 0 {
            out.push_str(&rest_in[..pos + WORD.len() + ws]);
            rest_in = &rest_in[pos + WORD.len() + ws..];
            rest_low = &rest_low[pos + WORD.len() + ws..];
            continue;
        }
        out.push_str(&rest_in[..pos + WORD.len() + ws]);
        out.push_str(MASK);
        rest_in = &rest_in[pos + WORD.len() + ws + value_len..];
        rest_low = &rest_low[pos + WORD.len() + ws + value_len..];
    }
    out.push_str(rest_in);
    out
}

/// Finds the earliest occurrence of any secret keyword at or after `from`.
fn next_secret_keyword(lower: &[char], from: usize) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for keyword in SECRET_KEYWORDS {
        let needle: Vec<char> = keyword.chars().collect();
        if let Some(off) = find_substring(lower, from, &needle) {
            let pos = from + off;
            match best {
                Some((best_pos, _)) if best_pos <= pos => {}
                _ => best = Some((pos, keyword.len())),
            }
        }
    }
    best
}

fn find_substring(haystack: &[char], from: usize, needle: &[char]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
}

/// Whether the identifier `key` names a secret-bearing key (S3).
fn is_secret_key(key: &[char]) -> bool {
    let key_str: String = key.iter().collect();
    let lower = key_str.to_ascii_lowercase();
    let last_segment = lower.split(['_', '-', '.']).next_back().unwrap_or(&lower);
    if SECRET_KEY_SEGMENTS.contains(&last_segment) {
        return true;
    }
    let normalized: String = lower
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | '.'))
        .collect();
    if SECRET_KEY_NORMS.contains(&normalized.as_str()) {
        return true;
    }
    SECRET_KEY_SUFFIXES
        .iter()
        .any(|suffix| normalized.len() > suffix.len() && normalized.ends_with(suffix))
}

fn expand_key_backward(chars: &[char], mut i: usize) -> usize {
    while i > 0 && is_key_char(chars[i - 1]) {
        i -= 1;
    }
    i
}

fn expand_key_forward(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && is_key_char(chars[i]) {
        i += 1;
    }
    i
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'
}

fn skip_spaces(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    i
}

/// Returns the end index (exclusive) of the value starting at `start`:
/// the closing quote for a quoted value, or the end of the bare token.
fn value_end(chars: &[char], start: usize) -> usize {
    let opening = chars.get(start);
    if matches!(opening, Some(&'"') | Some(&'\'')) {
        let quote = *opening.unwrap_or(&'"');
        let mut k = start + 1;
        while k < chars.len() {
            match chars[k] {
                '\\' if quote == '"' => k += 2,
                c if c == quote => return k + 1,
                _ => k += 1,
            }
        }
        return chars.len();
    }
    let mut k = start;
    while k < chars.len()
        && !chars[k].is_whitespace()
        && !matches!(
            chars[k],
            '{' | '}' | '[' | ']' | '(' | ')' | ',' | ';' | '"' | '\''
        )
    {
        k += 1;
    }
    k
}

/// Values that cannot be secrets are left untouched (keeps structured
/// payloads parseable and avoids redacting metadata).
fn is_inert_value(chars: &[char]) -> bool {
    let value: String = chars.iter().collect();
    let lower = value.to_ascii_lowercase();
    if lower == "null" || lower == "true" || lower == "false" {
        return true;
    }
    value.parse::<f64>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::mask_secrets;

    #[test]
    fn masks_sk_token() {
        assert_eq!(mask_secrets("sk-fake123456789"), "sk-****");
    }

    #[test]
    fn masks_token_embedded_in_text() {
        let masked = mask_secrets("failed: key=sk-fake123456789 retry");
        assert_eq!(masked, "failed: key=sk-**** retry");
    }

    #[test]
    fn leaves_short_prefixes_alone() {
        assert_eq!(
            mask_secrets("sk-123 and sk-abcde more"),
            "sk-123 and sk-abcde more"
        );
    }

    #[test]
    fn masks_multiple_tokens() {
        let masked = mask_secrets("a=sk-aaaaaaaa b=sk-bbbbbbbb");
        assert_eq!(masked, "a=sk-**** b=sk-****");
    }

    #[test]
    fn leaves_normal_unchanged() {
        assert_eq!(
            mask_secrets("openclaw 2026.7.1-2 status running"),
            "openclaw 2026.7.1-2 status running"
        );
    }

    // --- S3: API key / token / password / channel credential ---------------

    #[test]
    fn masks_unquoted_token_assignment() {
        assert_eq!(
            mask_secrets("retry with token=abcdef123456"),
            "retry with token=****"
        );
    }

    #[test]
    fn masks_api_key_variants() {
        assert_eq!(mask_secrets("APIKey: k1234567890"), "APIKey: ****");
        assert_eq!(mask_secrets("api_key=abcdef123"), "api_key=****");
        assert_eq!(mask_secrets("X-Api-Key:abcd1234"), "X-Api-Key:****");
    }

    #[test]
    fn masks_env_style_token_vars() {
        assert_eq!(
            mask_secrets("OPENCLAW_GATEWAY_TOKEN=supersecret123"),
            "OPENCLAW_GATEWAY_TOKEN=****"
        );
    }

    #[test]
    fn masks_json_quoted_secrets() {
        let masked = mask_secrets(r#"{"password":"hunter2!","token":"tok_abc123"}"#);
        assert_eq!(masked, r#"{"password":"****","token":"****"}"#);
    }

    #[test]
    fn masks_bearer_token() {
        assert_eq!(
            mask_secrets("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.payload.sig"),
            "Authorization: Bearer ****"
        );
    }

    #[test]
    fn leaves_bearer_word_without_value_alone() {
        assert_eq!(mask_secrets("the bearer"), "the bearer");
    }

    #[test]
    fn leaves_secret_named_words_without_value_alone() {
        assert_eq!(
            mask_secrets("no token was configured"),
            "no token was configured"
        );
    }

    #[test]
    fn leaves_inert_values_parseable() {
        assert_eq!(
            mask_secrets(r#"{"token": null,"timeoutMs": 3000}"#),
            r#"{"token": null,"timeoutMs": 3000}"#
        );
    }

    #[test]
    fn leaves_non_secret_keys_alone() {
        assert_eq!(
            mask_secrets("primaryTargetId: localLoopback, count: 3"),
            "primaryTargetId: localLoopback, count: 3"
        );
        assert_eq!(
            mask_secrets(r#"{"status":"running","secretfree":true}"#),
            r#"{"status":"running","secretfree":true}"#
        );
    }

    #[test]
    fn leaves_gateway_payload_fields_untouched() {
        let payload = r#""url":"ws://127.0.0.1:18789","capability":"read_only""#;
        assert_eq!(mask_secrets(payload), payload);
    }
}
