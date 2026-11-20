//! Interface tests for the language choice. They change the language on the test thread, open
//! Settings, and check the text the accessibility tree shows. Tests do not read system fonts, so
//! only the text is checked for Japanese. They use the helpers in `ui_tests`.

use crate::settings::Language;
use crate::ui_tests::*;

#[test]
fn settings_shows_the_text_of_the_chosen_language() {
    let repo = repository();
    let mut harness = open(repo.path());
    // The test harness panics on a character no font draws. The app draws a box instead, and
    // this test checks the words, so it allows boxes for the Japanese text.
    harness.ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Tofu);
    loaded(&mut harness);
    harness.state_mut().show_settings = true;
    settle(&mut harness);
    assert!(has_label(&harness, "GENERAL"), "English is the default in tests");

    let ctx = harness.ctx.clone();
    harness.state_mut().set_language(&ctx, Language::German);
    settle(&mut harness);
    assert!(has_label(&harness, "ALLGEMEIN"), "the German heading for General is shown");
    assert!(!has_label(&harness, "GENERAL"), "no English heading remains in German");
    assert!(has_label(&harness, "Sprache"), "the Language row is labelled in German");

    harness.state_mut().set_language(&ctx, Language::Japanese);
    settle(&mut harness);
    assert!(has_label(&harness, "一般"), "the Japanese heading for General is shown");
    assert!(has_label(&harness, "言語"), "the Language row is labelled in Japanese");

    harness.state_mut().set_language(&ctx, Language::English);
    settle(&mut harness);
    assert!(has_label(&harness, "GENERAL"), "English returns after switching back");
}
