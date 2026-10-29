use alexandria_zim::{article_html, html_to_text, internal_link};

#[test]
fn keeps_block_structure_and_skips_chrome() {
    let html = r#"<html><head><style>.x{color:red}</style></head><body>
      <div style="visibility:hidden">SECRET</div>
      <h1>Title</h1><p>First &amp; second.</p>
      <div class="mw-footer">footer junk</div>
      <ul><li>One</li><li>Two</li></ul>
      <p>Caf&eacute; &#233;t&#xe9;</p>
    </body></html>"#;
    let text = html_to_text(html);
    assert!(text.contains("Title"));
    assert!(text.contains("First & second."));
    assert!(text.contains("• One"));
    assert!(text.contains("• Two"));
    assert!(text.contains("Café été"));
    assert!(!text.contains("SECRET"));
    assert!(!text.contains("color:red"));
    assert!(!text.contains("footer junk"));
}

#[test]
fn does_not_corrupt_utf8() {
    let text = html_to_text("<p>Lokásá ya libosó</p>");
    assert_eq!(text, "Lokásá ya libosó");
}

#[test]
fn article_html_keeps_structure_and_drops_active_content() {
    let html = r#"<html><head><style>.x{color:red}</style><script>bad()</script></head>
      <body><nav>NAV JUNK</nav><div id="mw-content-text"><h2 id="History">History</h2>
      <p>Hello <b>world</b>. <a href="https://example.com" onclick="evil()">link</a></p>
      <ul><li>One</li><li>Two</li></ul><script>alert(1)</script></div></body></html>"#;
    let out = article_html(html);
    assert!(out.contains("<h2 id=\"History\">History</h2>"), "{out}");
    assert!(out.contains("<p>Hello <b>world</b>."), "{out}");
    assert!(out.contains("<li>One</li>"), "{out}");
    assert!(out.contains("link"), "{out}");
    assert!(!out.contains("<script"), "{out}");
    assert!(!out.contains("alert"), "{out}");
    assert!(!out.contains("href="), "{out}");
    assert!(!out.contains("onclick"), "{out}");
    assert!(!out.contains("NAV JUNK"), "{out}");
    assert!(!out.contains("color:red"), "{out}");
}

#[test]
fn article_links_become_fragment_targets() {
    let html = r##"<div id="mw-content-text"><p>
      <a href="Kinshasa" title="Kinshasa">capital</a>,
      <a href="./Congo_River#Course">river</a>,
      <a href="../A/S%C3%A3o_Paulo">city</a>,
      <a href="https://example.com/x">web</a>,
      <a href="//evil.example/y">proto</a>,
      <a href="javascript:alert(1)">js</a>,
      <a href="#cite_note-1">note</a>,
      <a href='Bad"onmouseover="x'>quote</a>
    </p></div>"##;
    let out = article_html(html);
    let hrefs: Vec<&str> = out.match_indices("href=\"").map(|(i, _)| &out[i + 6..i + 9]).collect();
    assert!(hrefs.iter().all(|h| *h == "#a:"), "{out}");
    assert!(out.contains(r##"<a class="al" href="#a:Kinshasa">capital</a>"##), "{out}");
    assert!(out.contains(r##"href="#a:Congo_River">river</a>"##), "{out}");
    assert!(out.contains(r##"href="#a:S%C3%A3o_Paulo">city</a>"##), "{out}");
    assert!(out.contains(r##"<a class="ext">web</a>"##), "{out}");
    assert!(out.contains(r##"<a class="ext">proto</a>"##), "{out}");
    assert!(out.contains(r##"<a class="ext">js</a>"##), "{out}");
    assert!(out.contains(r##"<a class="ext">note</a>"##), "{out}");
    assert!(out.contains(r##"href="#a:Bad%22onmouseover%3D%22x">quote</a>"##), "{out}");
    assert!(!out.contains("javascript"), "{out}");
}

#[test]
fn internal_link_handles_odd_escapes() {
    assert_eq!(internal_link("a%é%C3%A9%"), Some("a%éé%".to_string()));
    assert_eq!(internal_link("%"), Some("%".to_string()));
    assert_eq!(internal_link("mailto:x@y"), None);
    assert_eq!(internal_link("C/Paris"), Some("Paris".to_string()));
    assert_eq!(internal_link("Paris?x=1"), Some("Paris".to_string()));
    assert_eq!(internal_link("./"), None);
}
