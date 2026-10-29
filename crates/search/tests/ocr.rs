use alexandria_search::ocr::{Ocr, HEADER};

fn sample() -> String {
    let paths = [("files/Knots (7).pdf", 2971688u64), ("files/Water (1).pdf", 12u64)];
    let texts = ["A coil made up for stowing.\nTo break a heavy cord, take a turn.\n".to_string(), "@@ looks like a header\nsecond line\n".to_string()];
    Ocr::default().to_text(&paths, &texts)
}

#[test]
fn text_written_and_read_back_per_file() {
    let file = sample();
    assert!(file.starts_with(HEADER));
    let ocr = Ocr::parse(file.as_bytes());
    assert_eq!(ocr.len(), 2);
    let knots = ocr.text("files/Knots (7).pdf").unwrap();
    assert!(knots.contains("take a turn"));
    let water = ocr.text("files/Water (1).pdf").unwrap();
    assert!(water.starts_with("@@ looks like a header"), "{water}");
    assert!(water.contains("second line"));
}

#[test]
fn a_file_that_is_not_ocr_data_yields_nothing() {
    assert!(Ocr::parse(b"random bytes\n@@ x 1\nhello").is_empty());
    assert!(Ocr::parse(b"").is_empty());
}

#[test]
fn empty_or_tiny_text_is_not_offered() {
    let ocr = Ocr::parse(b"alexandria-ocr 1\n@@ a.pdf 1\nshort\n");
    assert!(ocr.text("a.pdf").is_none());
    assert!(ocr.text("missing.pdf").is_none());
}
