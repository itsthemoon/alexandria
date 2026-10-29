pub const NOT_IN_LIBRARY: &str = "The library excerpts do not cover that.";

fn clean(s: &str) -> String {
    s.chars().filter(|&c| !c.is_control() || c == '\n' || c == '\t').collect()
}

pub fn compact(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for line in s.lines() {
        let mut l = String::new();
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            let mut j = i;
            while j < chars.len() && chars[j] == c {
                j += 1;
            }
            if c.is_whitespace() {
                l.push(' ');
            } else if j - i >= 4 && c.is_ascii_punctuation() {
                l.push(' ');
            } else {
                chars[i..j].iter().for_each(|&c| l.push(c));
            }
            i = j;
        }
        let l = l.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
        if !l.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&l);
        }
    }
    out
}

pub fn ask_prompt(context: &str, question: &str) -> (String, String) {
    let (context, question) = (compact(&clean(context)), clean(question));
    let system = format!(
        "You are Alexandria, an offline reference assistant. You answer only from the \
         numbered library excerpts below.\n\n{context}"
    );
    let user = format!(
        "{question}\n\nAnswer from the excerpts only, in at most five short sentences. Do not \
         think out loud or quote at length. If the excerpts do not contain the answer, reply \
         with exactly: {NOT_IN_LIBRARY}"
    );
    (system, user)
}

pub fn translate_prompt(target: &str, text: &str) -> (String, String) {
    let (target, text) = (clean(target), clean(text));
    let system = "You are a translator. You output only the translation.".to_string();
    let user = format!(
        "Translate into {target}. Detect the source language yourself. Keep names, numbers, \
         line breaks and meaning. Reply with the translation only, without quotation marks \
         or notes.\n\nText:\n{text}"
    );
    (system, user)
}

pub fn visible_answer(raw: &str) -> String {
    let text = match raw.rfind("</think>") {
        Some(i) => raw[i + "</think>".len()..].trim(),
        None => raw.trim(),
    };
    let rest = text.replace(NOT_IN_LIBRARY, "");
    let rest = rest.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    let rest = rest.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    let quote = |l: &str| l.trim_start().strip_prefix('[').and_then(|r| r.split_once(']')).is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    let lines: Vec<&str> = rest.lines().collect();
    let start = lines.iter().position(|l| !l.trim().is_empty() && !quote(l)).unwrap_or(lines.len());
    let rest = if start < lines.len() { lines[start..].join("\n") } else { rest.clone() };
    let paras: Vec<&str> = rest.split("\n\n").collect();
    let doubt = |p: &str| {
        let p = p.to_lowercase();
        p.contains("excerpts do not") || p.contains("excerpts don't")
    };
    let rest = match paras.iter().skip(1).position(|p| doubt(p)) {
        Some(i) => paras[..i + 1].join("\n\n"),
        None => rest.clone(),
    };
    let mut rest = rest.trim().to_string();
    if rest.ends_with(':') {
        if let Some(i) = rest[..rest.len() - 1].rfind(['.', '!', '?']) {
            rest.truncate(i + 1);
        }
    }
    let rest = rest.trim();
    if rest.is_empty() { text.to_string() } else { rest.to_string() }
}

pub fn streaming_view(partial: &str) -> Option<String> {
    (partial.contains("</think>") || partial.chars().count() >= 400).then(|| visible_answer(partial))
}

pub const CHAT_TEMPLATE: &str = include_str!("../assets/lfm3-chat.jinja");

