//! Output truncation helpers.
//!
//! AI agents have finite context windows, and command output can be arbitrarily
//! large (e.g. `find /`, `cat big_log.txt`).  Middle-omission truncation is
//! preferred over head/tail-only because it preserves both the beginning
//! (command invocation context) and the end (final result / error) of the
//! output.

/// Default maximum characters before truncation kicks in (30 000).
pub const DEFAULT_MAX_CHARS: usize = 30_000;

/// Truncate `output` to at most `max_chars` characters using middle-omission.
///
/// If `output` fits within `max_chars`, it is returned as-is (no allocation).
/// Otherwise the result is:
///
/// ```text
/// <first max_chars/2 chars>
///
/// ... [OUTPUT TRUNCATED: N chars omitted] ...
///
/// <last max_chars/2 chars>
/// ```
///
/// The split is done on character boundaries (not bytes), so multi-byte
/// Unicode sequences are never broken.
pub fn truncate_output(output: &str, max_chars: usize) -> String {
    if output.len() <= max_chars {
        // Fast path: no allocation needed when already within budget.
        return output.to_owned();
    }

    // Use character-aware splitting so we never slice across a multi-byte char.
    let chars: Vec<char> = output.chars().collect();
    let total = chars.len();

    if total <= max_chars {
        // The byte length was large but char count fits — return as-is.
        return output.to_owned();
    }

    let half = max_chars / 2;
    let head: String = chars[..half].iter().collect();
    let tail: String = chars[total - half..].iter().collect();
    let omitted = total - max_chars;

    format!("{head}\n\n... [OUTPUT TRUNCATED: {omitted} chars omitted] ...\n\n{tail}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_input_returned_as_is() {
        let s = "hello world";
        let result = truncate_output(s, DEFAULT_MAX_CHARS);
        assert_eq!(result, s);
    }

    #[test]
    fn exact_boundary_returned_as_is() {
        // A string of exactly max_chars should NOT be truncated.
        let s: String = "x".repeat(DEFAULT_MAX_CHARS);
        let result = truncate_output(&s, DEFAULT_MAX_CHARS);
        assert_eq!(result.len(), DEFAULT_MAX_CHARS);
        assert!(!result.contains("TRUNCATED"));
    }

    #[test]
    fn one_over_boundary_is_truncated() {
        let s: String = "a".repeat(DEFAULT_MAX_CHARS + 1);
        let result = truncate_output(&s, DEFAULT_MAX_CHARS);
        assert!(result.contains("OUTPUT TRUNCATED"));
        assert!(result.contains("1 chars omitted"));
    }

    #[test]
    fn large_input_preserves_head_and_tail() {
        // Build a recognisable string: 10k 'A's, 20k 'B's, 10k 'C's (total 40k).
        let head_marker: String = "A".repeat(10_000);
        let body: String = "B".repeat(20_000);
        let tail_marker: String = "C".repeat(10_000);
        let input = format!("{head_marker}{body}{tail_marker}");

        let result = truncate_output(&input, DEFAULT_MAX_CHARS);

        assert!(result.contains("OUTPUT TRUNCATED"));
        // The very first character must still be 'A'.
        assert!(result.starts_with('A'));
        // The very last character must still be 'C'.
        assert!(result.ends_with('C'));
    }

    #[test]
    fn custom_max_chars_respected() {
        let s: String = "z".repeat(200);
        let result = truncate_output(&s, 100);
        assert!(result.contains("OUTPUT TRUNCATED"));
        assert!(result.contains("100 chars omitted"));
    }

    #[test]
    fn unicode_does_not_break_on_boundary() {
        // Each '€' is 3 bytes.  We construct a string that is > max_chars bytes
        // but whose truncation points fall on valid char boundaries.
        let s: String = "€".repeat(40_000);
        // Should not panic.
        let result = truncate_output(&s, DEFAULT_MAX_CHARS);
        assert!(result.contains("OUTPUT TRUNCATED"));
    }

    #[test]
    fn empty_input_returned_as_is() {
        let result = truncate_output("", DEFAULT_MAX_CHARS);
        assert_eq!(result, "");
    }
}
