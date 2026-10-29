use alexandria_search::{pdf_text, pdf_title, pdf_titles, snippet};
use pdf_extract::{Dictionary, Document, Object, Stream};

fn pdf_with(text: &str) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type1".to_vec()));
    font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
    let font_id = doc.add_object(font);
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Reference(font_id));
    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(fonts));
    let resources_id = doc.add_object(resources);
    let body = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
    let content_id = doc.add_object(Stream::new(Dictionary::new(), body.into_bytes()));
    let mut page = Dictionary::new();
    page.set("Type", Object::Name(b"Page".to_vec()));
    page.set("Parent", Object::Reference(pages_id));
    page.set("Contents", Object::Reference(content_id));
    page.set("Resources", Object::Reference(resources_id));
    page.set("MediaBox", Object::Array(vec![0.into(), 0.into(), 612.into(), 792.into()]));
    let page_id = doc.add_object(page);
    let mut pages = Dictionary::new();
    pages.set("Type", Object::Name(b"Pages".to_vec()));
    pages.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
    pages.set("Count", Object::Integer(1));
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = doc.add_object(catalog);
    doc.trailer.set("Root", Object::Reference(catalog_id));
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

#[test]
fn pdf_text_reads_the_words_in_the_pdf_itself() {
    let (_, text) = pdf_text(&pdf_with("Boil water for one full minute")).unwrap();
    assert!(text.contains("Boil water for one full minute"), "{text:?}");
}

#[test]
fn pdf_text_refuses_bytes_that_are_not_a_pdf() {
    assert!(pdf_text(b"not a pdf at all").is_none());
}

#[test]
fn pdf_titles_come_from_each_verified_pdf_in_order() {
    let items = vec![
        ("files/Water (2).pdf".to_string(), pdf_with("Purifying Water During an Emergency")),
        ("files/Knots (1).pdf".to_string(), b"not a pdf".to_vec()),
        ("files/Fire (3).pdf".to_string(), pdf_with("Building a Safe Cooking Fire")),
    ];
    assert_eq!(
        pdf_titles(items),
        vec!["Purifying Water During an Emergency", "Knots (1)", "Building a Safe Cooking Fire"]
    );
}

#[test]
fn a_clean_metadata_title_wins() {
    assert_eq!(
        pdf_title(Some("GIARDIA: DRINKING WATER FACT SHEET"), "-1- United States\n...", "files/Water (3).pdf"),
        "GIARDIA: DRINKING WATER FACT SHEET"
    );
}

#[test]
fn urls_filenames_and_mojibake_fall_back_to_the_first_real_line() {
    let text = "\n  \nPurifying Water During an Emergency\nThe treatments described below...";
    for meta in [
        Some("http://www.doh.wa.gov/Emergencies/EmergencyPreparedness"),
        Some("Water (2).pdf"),
        Some("\u{fffd}\u{fffd}\0w\0q"),
        Some("Microsoft Word - Water.doc"),
        None,
    ] {
        assert_eq!(pdf_title(meta, text, "files/Water (2).pdf"), "Purifying Water During an Emergency", "{meta:?}");
    }
}

#[test]
fn a_long_first_line_is_cut_at_a_word() {
    let t = pdf_title(None, "Distillation is one of the oldest methods of water treatment and is still in use today, though not commonly", "x.pdf");
    assert!(t.len() <= 80 && t.starts_with("Distillation is one of the oldest") && !t.ends_with(' '), "{t}");
}

#[test]
fn no_text_keeps_a_readable_file_name() {
    assert_eq!(pdf_title(None, "", "files/Emergency Preparedness (1).pdf"), "Emergency Preparedness (1)");
}

#[test]
fn a_snippet_is_cut_around_the_first_matching_word() {
    let text = format!("{} Boiling water kills most germs that cause disease. {}", "Intro words here. ".repeat(30), "More text follows. ".repeat(30));
    let s = snippet(&text, "boil water").unwrap();
    assert!(s.contains("Boiling water kills most germs"), "{s}");
    assert!(s.chars().count() <= 200, "{}", s.chars().count());
    assert!(s.starts_with('…') && s.ends_with('…'), "{s}");
}

#[test]
fn a_snippet_at_the_start_has_no_leading_ellipsis() {
    let s = snippet("Water purification is the process of removing contaminants from water.", "water").unwrap();
    assert!(s.starts_with("Water purification"), "{s}");
}

#[test]
fn no_matching_word_means_no_snippet() {
    assert_eq!(snippet("Nothing relevant in this text at all.", "malaria"), None);
}
