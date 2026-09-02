use unicode_linebreak::BreakOpportunity;

use super::LineWrapper;

/// Returns every Unicode line-break opportunity as the byte index immediately
/// after the preceding character.
///
/// GPUI's product-level word tailoring (`LineWrapper::is_word_char`) decides
/// where a soft wrap is desirable; UAX #14 is then consulted as a veto whenever
/// a non-ASCII, non-word character participates in the boundary so CJK,
/// emoji, and punctuation clusters stay legal. Keeping one word table shared
/// with `LineWrapper` means upstream tailoring changes flow into this policy
/// without a second list to maintain.
pub(crate) fn opportunities(text: &str) -> impl Iterator<Item = (usize, BreakOpportunity)> + '_ {
    let unicode_opportunities = unicode_linebreak::linebreaks(text).collect::<Vec<_>>();
    let mut tailored_opportunities = Vec::new();
    let mut previous = None;

    for (offset, character) in text.char_indices() {
        let legacy_allowed = match previous {
            Some(previous) if LineWrapper::is_word_char(character) => previous == ' ',
            Some(_) => character != ' ',
            None => false,
        };

        if legacy_allowed
            && (!requires_unicode_validation(previous, character)
                || unicode_opportunities
                    .binary_search_by_key(&offset, |(offset, _)| *offset)
                    .is_ok())
        {
            tailored_opportunities.push((offset, BreakOpportunity::Allowed));
        }

        previous = Some(character);
    }

    // Preserve mandatory breaks (including end-of-text) independently of the
    // product-specific soft-wrap tailoring above.
    tailored_opportunities.extend(
        unicode_opportunities
            .into_iter()
            .filter(|(_, opportunity)| *opportunity == BreakOpportunity::Mandatory),
    );
    tailored_opportunities.sort_unstable_by_key(|(offset, opportunity)| {
        (*offset, *opportunity == BreakOpportunity::Allowed)
    });
    tailored_opportunities.dedup_by_key(|(offset, _)| *offset);
    tailored_opportunities.into_iter()
}

/// Returns the byte offsets at which a line may (or must) end.
pub(crate) fn offsets(text: &str) -> impl Iterator<Item = usize> + '_ {
    opportunities(text).map(|(offset, _)| offset)
}

/// UAX #14 is used as a veto when non-ASCII, non-word content participates in
/// a boundary. This fixes CJK/emoji punctuation and cluster rules while the
/// existing ASCII product tailoring remains byte-for-byte compatible.
fn requires_unicode_validation(previous: Option<char>, character: char) -> bool {
    fn is_unicode_sensitive(character: char) -> bool {
        character != '\u{fffc}' && !character.is_ascii() && !LineWrapper::is_word_char(character)
    }

    previous.is_some_and(is_unicode_sensitive) || is_unicode_sensitive(character)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed_offsets(text: &str) -> Vec<usize> {
        opportunities(text)
            .filter_map(|(offset, opportunity)| {
                (opportunity == BreakOpportunity::Allowed).then_some(offset)
            })
            .collect()
    }

    #[test]
    fn keeps_cjk_closing_punctuation_with_the_preceding_text() {
        let text = "你好，世界";
        let breaks = allowed_offsets(text);

        assert!(!breaks.contains(&"你好".len()), "must not break before ，");
        assert!(breaks.contains(&"你好，".len()), "may break after ，");
    }

    #[test]
    fn observes_cjk_opening_and_closing_punctuation_rules() {
        let text = "（内容）";
        let breaks = allowed_offsets(text);

        assert!(!breaks.contains(&"（".len()), "must not break after （");
        assert!(
            breaks.contains(&"（内".len()),
            "may break between ideographs"
        );
        assert!(
            !breaks.contains(&"（内容".len()),
            "must not break before ）"
        );
    }

    #[test]
    fn breaks_between_cjk_ideographs_but_not_inside_latin_words() {
        assert!(allowed_offsets("你好").contains(&"你".len()));
        assert!(allowed_offsets("hello").is_empty());
    }

    #[test]
    fn breaks_after_spaces_and_reports_the_end_as_mandatory() {
        assert!(allowed_offsets("hello world").contains(&"hello ".len()));
        assert_eq!(
            opportunities("hello")
                .filter(|(_, opportunity)| *opportunity == BreakOpportunity::Mandatory)
                .collect::<Vec<_>>(),
            vec![("hello".len(), BreakOpportunity::Mandatory)]
        );
    }

    #[test]
    fn preserves_gpui_ascii_word_and_url_tailoring() {
        for word in [
            "Hello123",
            "non-English",
            "var_name",
            "3.1415",
            "10^2",
            "1~2",
            "100%",
            "@mention",
            "#hashtag",
            "$variable",
            "a=1",
            "Self::new",
            "on;",
            "more⋯",
            "won’t",
            "‘twas",
            "github.com",
        ] {
            assert!(
                allowed_offsets(word).is_empty(),
                "legacy ASCII token unexpectedly wrapped: {word}"
            );
        }
        assert_eq!(
            allowed_offsets("zed-industries/zed"),
            vec!["zed-industries".len()]
        );
        assert_eq!(
            allowed_offsets("zed-industries\\zed"),
            vec!["zed-industries".len()]
        );
        assert_eq!(allowed_offsets("a=1&b=2"), vec!["a=1".len()]);
        assert_eq!(allowed_offsets("foo?b=2"), vec!["foo".len()]);
    }

    #[test]
    fn preserves_gpui_words_in_previously_supported_scripts() {
        for word in [
            "ÀÁÂÃÄÅÆÇÈÉÊËÌÍÎÏ",
            "ĀāĂăĄąĆćĈĉĊċČčĎď",
            "ƀƁƂƃƄƅƆƇƈƉƊƋƌƍƎƏ",
            "АБВГДЕЖЗИЙКЛМНОП",
            "ThậmchíđếnkhithuachạychúngcònnhẫntâmgiếtnốtsốđôngtùchínhtrịởYênBáivàCaoBằng",
            "গিয়েছিলেন",
            "ছেলে",
            "হচ্ছিল",
        ] {
            assert!(
                allowed_offsets(word).is_empty(),
                "legacy word unexpectedly wrapped: {word}"
            );
        }
    }

    #[test]
    fn preserves_non_breaking_glue_sequences() {
        assert!(allowed_offsets("a\u{202f}b\u{00a0}c\u{2011}d").is_empty());
    }

    #[test]
    fn keeps_upstream_closing_punctuation_attached_to_the_preceding_word() {
        // UAX #14 LB13 tailoring from `LineWrapper::is_word_char`: closing
        // punctuation never starts a wrapped line.
        for word in [
            "plz!",
            "see)",
            "list]",
            "block}",
            "said\"",
            "quoted”",
            "quoted»",
            "well…",
            "foo)bar",
            "x!y",
        ] {
            assert!(
                allowed_offsets(word).is_empty(),
                "closing punctuation unexpectedly wrapped: {word}"
            );
        }

        // An opening quote after a space still starts a new word.
        assert_eq!(
            allowed_offsets("he said \"hi\""),
            vec!["he ".len(), "he said ".len()]
        );
        // Slashes and question marks remain break opportunities for paths/URLs.
        assert_eq!(allowed_offsets("a/b?c"), vec!["a".len(), "a/b".len()]);
    }
}
