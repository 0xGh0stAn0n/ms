//! The player's language, as the phone page reports it (a locale such as
//! `he-IL`): its name for the model, and whether MapleSyrup's own lines
//! (warnings, greetings) must be translated before they are spoken.

/// The languages the phone's picker offers, by its codes (the page's
/// `LANGUAGES`): what MapleSyrup can be switched to when the player asks.
pub const LOCALES: &[&str] = &[
    "en-US", "en-GB", "he-IL", "es-ES", "es-MX", "pt-BR", "fr-FR", "de-DE", "ko-KR", "ja-JP",
    "zh-CN", "zh-TW", "th-TH", "vi-VN", "id-ID", "ru-RU",
];

/// The language's name in English, for the model's instructions.
pub fn name(locale: &str) -> &'static str {
    let lower = locale.trim().to_ascii_lowercase();
    match lower.as_str() {
        "es-mx" | "es-419" | "es-us" => return "Latin American Spanish",
        "pt-pt" => return "European Portuguese",
        "zh-tw" | "zh-hk" | "zh-hant" => return "Traditional Chinese",
        _ => {}
    }
    match lower.split(['-', '_']).next().unwrap_or("") {
        "he" | "iw" => "Hebrew",
        "es" => "Spanish",
        "pt" => "Brazilian Portuguese",
        "fr" => "French",
        "de" => "German",
        "ko" => "Korean",
        "ja" => "Japanese",
        "zh" => "Simplified Chinese",
        "th" => "Thai",
        "vi" => "Vietnamese",
        "id" | "in" => "Indonesian",
        "ru" => "Russian",
        "ar" => "Arabic",
        "it" => "Italian",
        "pl" => "Polish",
        "tr" => "Turkish",
        "uk" => "Ukrainian",
        "nl" => "Dutch",
        "ms" => "Malay",
        "tl" | "fil" => "Filipino",
        _ => "English",
    }
}

/// Whether MapleSyrup's own (English) lines can be spoken as they are.
pub fn is_english(locale: &str) -> bool {
    name(locale) == "English"
}

/// The language as an ISO-639-1 code, for a transcriber told what language
/// to expect (`he-IL` → `he`; the old codes some phones still send, `iw`
/// and `in`, are today's `he` and `id`). None when the locale names no
/// two-letter language.
pub fn code(locale: &str) -> Option<String> {
    let primary = locale
        .trim()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let primary = match primary.as_str() {
        "iw" => "he".to_string(),
        "in" => "id".to_string(),
        "fil" => "tl".to_string(),
        _ => primary,
    };
    (primary.len() == 2 && primary.chars().all(|c| c.is_ascii_lowercase())).then_some(primary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_have_names() {
        assert_eq!(name("he-IL"), "Hebrew");
        assert_eq!(name("es-MX"), "Latin American Spanish");
        assert_eq!(name("es-ES"), "Spanish");
        assert_eq!(name("zh-TW"), "Traditional Chinese");
        assert_eq!(name("zh-CN"), "Simplified Chinese");
        assert_eq!(name("pt-BR"), "Brazilian Portuguese");
        assert!(is_english("en-GB"));
        assert!(is_english(""));
        assert!(!is_english("ko-KR"));
    }

    #[test]
    fn the_languages_offered_are_the_phones_pickers() {
        for locale in LOCALES {
            assert!(
                crate::phone::PAGE.contains(&format!("[\"{locale}\", ")),
                "the phone's picker lacks {locale}"
            );
        }
        let picker = crate::phone::PAGE
            .split("const LANGUAGES = [")
            .nth(1)
            .and_then(|rest| rest.split("];").next())
            .unwrap_or("");
        assert_eq!(picker.matches("[\"").count(), LOCALES.len(), "{picker}");
    }

    #[test]
    fn locales_have_codes_for_a_transcriber() {
        assert_eq!(code("he-IL").as_deref(), Some("he"));
        assert_eq!(code("iw").as_deref(), Some("he"));
        assert_eq!(code("in-ID").as_deref(), Some("id"));
        assert_eq!(code("fil-PH").as_deref(), Some("tl"));
        assert_eq!(code("EN_us").as_deref(), Some("en"));
        assert_eq!(code(""), None);
        assert_eq!(code("x1-YY"), None);
        assert_eq!(code("haw-US"), None);
    }
}
