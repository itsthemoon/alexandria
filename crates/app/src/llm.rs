use alexandria_core::{Entry, Manifest};

pub fn pick_model(m: &Manifest) -> Option<Entry> {
    m.entries.iter().find(|e| e.path.ends_with(".gguf")).cloned()
}

pub fn model_label(path: &str) -> &'static str {
    if path.to_ascii_lowercase().starts_with("lfm3") {
        "LFM3 1.7B"
    } else {
        "local model"
    }
}

pub use crate::prompts::{ask_prompt, compact, streaming_view, translate_prompt, visible_answer, CHAT_TEMPLATE};
#[cfg(test)]
use crate::prompts::NOT_IN_LIBRARY;

pub struct Stream {
    buf: Vec<u8>,
    done: bool,
}

const END: &[u8] = b" [end of text]";

impl Stream {
    pub fn new() -> Stream {
        Stream { buf: Vec::new(), done: false }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> String {
        if self.done {
            return String::new();
        }
        self.buf.extend_from_slice(bytes);
        if let Some(p) = find(&self.buf, END) {
            let out = String::from_utf8_lossy(&self.buf[..p]).to_string();
            self.buf.clear();
            self.done = true;
            return out;
        }
        let mut safe = self.buf.len().saturating_sub(END.len() - 1);
        while safe > 0 && safe < self.buf.len() && (self.buf[safe] & 0xC0) == 0x80 {
            safe -= 1;
        }
        let ready: Vec<u8> = self.buf.drain(..safe).collect();
        String::from_utf8_lossy(&ready).to_string()
    }

    pub fn finish(&mut self) -> String {
        let rest = if self.done {
            String::new()
        } else {
            String::from_utf8_lossy(&self.buf).trim_end().to_string()
        };
        self.buf.clear();
        self.done = true;
        rest
    }
}

pub fn template_file(dir: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in CHAT_TEMPLATE.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let path = dir.join(format!("lfm3-chat-{h:016x}.jinja"));
    if std::fs::read(&path).ok().as_deref() != Some(CHAT_TEMPLATE.as_bytes()) {
        std::fs::create_dir_all(dir)?;
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, CHAT_TEMPLATE)?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(path)
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "Où est-il ?\nLa première ligne, délicate. [end of text]\n\n\n";

    fn run(raw: &str, chunk: usize) -> String {
        let mut s = Stream::new();
        let mut out = String::new();
        for c in raw.as_bytes().chunks(chunk) {
            out.push_str(&s.feed(c));
        }
        out.push_str(&s.finish());
        out
    }

    #[test]
    fn stops_at_end_of_text_at_any_chunking() {
        for n in [1, 2, 3, 7, 64, 4096] {
            assert_eq!(run(RAW, n), "Où est-il ?\nLa première ligne, délicate.", "chunk {n}");
        }
    }

    #[test]
    fn keeps_everything_when_the_token_cap_ends_it() {
        for n in [1, 5, 4096] {
            assert_eq!(run("Cut off mid-sent\n", n), "Cut off mid-sent", "chunk {n}");
        }
    }

    #[test]
    fn chat_template_has_thinking_off() {
        assert!(CHAT_TEMPLATE.contains("{%- set enable_thinking = false -%}"));
        let dir = std::env::temp_dir().join(format!("alex-tpl-{}", std::process::id()));
        let p = template_file(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), CHAT_TEMPLATE);
        assert_eq!(template_file(&dir).unwrap(), p);
    }

    #[test]
    fn picks_the_library_model() {
        let z = "0".repeat(64);
        let m = Manifest::parse(&format!(
            "alexandria-manifest 1\nrelease v1 2026-10-27\nchunk 1048576\n{z} 1 a.zim\n{z} 1 lfm3-1.7b.gguf\n"
        ))
        .unwrap();
        assert_eq!(pick_model(&m).unwrap().path, "lfm3-1.7b.gguf");
        assert_eq!(model_label("lfm3-1.7b.gguf"), "LFM3 1.7B");
    }

    #[test]
    fn translate_instruction_travels_in_the_user_message() {
        let (system, user) = translate_prompt("Spanish", "Where is the hospital?");
        assert!(user.starts_with("Translate into Spanish."));
        assert!(user.ends_with("Text:\nWhere is the hospital?"));
        assert!(!system.contains("Where is the hospital?"));
    }

    #[test]
    fn ask_prompt_caps_length_and_names_the_refusal() {
        let (system, user) = ask_prompt("[1] Water\nWater is wet.", "Is water wet?");
        assert!(system.contains("[1] Water\nWater is wet."));
        assert!(user.contains("Is water wet?"));
        assert!(user.contains(NOT_IN_LIBRARY));
    }

    #[test]
    fn reasoning_the_model_leaks_before_a_closing_think_tag_is_not_shown() {
        assert_eq!(visible_answer("Let's craft:\n1. boil\nThat's it.</think>Boil the water."), "Boil the water.");
        assert_eq!(visible_answer("a</think>b</think>\n\nFinal."), "Final.");
        assert_eq!(visible_answer("  Plain answer.\n"), "Plain answer.");
        assert_eq!(visible_answer("Still reasoning, no tag yet"), "Still reasoning, no tag yet");
    }

    #[test]
    fn the_not_covered_line_is_dropped_when_there_is_a_real_answer() {
        assert_eq!(visible_answer("Boil it for one minute.\n\nThe library excerpts do not cover that."), "Boil it for one minute.");
        assert_eq!(visible_answer("Boil it. The library excerpts do not cover that. Then cool it."), "Boil it. Then cool it.");
        assert_eq!(visible_answer("The library excerpts do not cover that."), "The library excerpts do not cover that.");
    }

    #[test]
    fn streamed_text_is_held_until_leaked_reasoning_is_ruled_out() {
        let leak = "Where is the nearest hospital?\nThe user wrote in Spanish: \"¿Dónde está el hospital más cercano?\" which translates to \"Where is the neare";
        assert_eq!(streaming_view(leak), None);
        assert_eq!(streaming_view(&format!("{leak}\"</think>Where is the nearest hospital?")).as_deref(), Some("Where is the nearest hospital?"));
        let long = "Boil the water for one minute. ".repeat(20);
        assert_eq!(streaming_view(&long).as_deref(), Some(long.trim()));
    }

    #[test]
    fn a_second_guessing_tail_after_a_real_answer_is_cut() {
        let raw = "Drink clean water and oral rehydration solution.\n\nThe library excerpts do not contain the answer? No, here it is in short sentences:\n\n- Drink clean water.";
        assert_eq!(visible_answer(raw), "Drink clean water and oral rehydration solution.");
        assert_eq!(visible_answer("The excerpts do not contain treatment steps."), "The excerpts do not contain treatment steps.");
    }

    #[test]
    fn quoted_excerpt_lines_before_the_answer_are_dropped() {
        let raw = "[1] Burns: \"Cool the burn with water.\"\n[13] Zones: coagulation.\n\nCool the burn under running water for 20 minutes.";
        assert_eq!(visible_answer(raw), "Cool the burn under running water for 20 minutes.");
        assert_eq!(visible_answer("Cool it [1] and cover it [2]."), "Cool it [1] and cover it [2].");
    }

    #[test]
    fn an_answer_that_stops_on_a_colon_loses_the_unfinished_part() {
        let raw = "The excerpts discuss snake bites but give no treatment steps. Therefore:";
        assert_eq!(visible_answer(raw), "The excerpts discuss snake bites but give no treatment steps.");
        assert_eq!(visible_answer("Do this:"), "Do this:");
    }

    #[test]
    fn excerpts_lose_pdf_layout_padding_but_keep_their_words() {
        let raw = "\n\n-1-\n\nUnited States          Office of Water\n   ________________\n\n    GIARDIA: FACT SHEET   \n \n\n\nWhat is Giardia?\nGiardia are  parasites... see p. 3";
        let (s, _) = ask_prompt(raw, "What is giardia?");
        assert!(s.contains("United States Office of Water\nGIARDIA: FACT SHEET\nWhat is Giardia?\nGiardia are parasites... see p. 3"), "{s:?}");
        assert!(!s.contains("___") && !s.contains("  ") && !s.contains("\n\n\n"));
    }

    #[test]
    fn prompts_never_carry_nul_or_control_bytes_into_the_runner() {
        let (s, u) = ask_prompt("[1] Pdf\n%PDF-1.4\0\x01\x07binary\tok", "why\0 boil?\x1b");
        for p in [&s, &u] {
            assert!(!p.chars().any(|c| c.is_control() && c != '\n' && c != '\t'), "{p:?}");
        }
        assert!(s.contains("%PDF-1.4binary ok"));
        assert!(u.contains("why boil?"));
        let (s, u) = translate_prompt("Spanish", "hola\0 mundo");
        assert!(!s.contains('\0') && !u.contains('\0'));
        assert!(u.ends_with("hola mundo"));
    }

    #[test]
    #[ignore]
    fn live_model() {
        let llama = std::env::var("ALEX_TEST_LLAMA").expect("ALEX_TEST_LLAMA");
        let model = std::path::PathBuf::from(std::env::var("ALEX_TEST_MODEL").expect("ALEX_TEST_MODEL"));
        let go = |system: &str, user: &str, n: u32, temp: &str| {
            let cmd = std::process::Command::new(&llama);
            let tpl = template_file(&std::env::temp_dir().join("alex-live")).unwrap();
            super::run(cmd, &model, &tpl, system, user, n, temp, next_generation(), &mut |_| {}).unwrap()
        };
        let mut fails = Vec::new();
        let mut check = |name: &str, out: &str, ok: bool| {
            println!("{} {name}\n    -> {}", if ok { "PASS" } else { "FAIL" }, out.trim());
            if !ok {
                fails.push(name.to_string());
            }
        };

        let thin = "[1] Water purification\nWater purification is the process of removing \
                    contaminants from water. One reason is to make it safe for humans to consume. \
                    Another is industrial use such as cleaning and cooling microchips.";
        let (s, u) = ask_prompt(thin, "Name two ways to make water safe to drink.");
        let out = go(&s, &u, 300, "0.1");
        check("refuses when excerpts lack the answer", &out,
            out.contains(NOT_IN_LIBRARY) && out.len() < 200);

        let rich = "[1] Dysentery\nDysentery is a disease that involves severe bloody diarrhea. \
                    It is caused by a bacterium, which causes the intestines to swell up. It is \
                    usually treated with antibiotics and by drinking plenty of fluids.";
        let (s, u) = ask_prompt(rich, "What causes dysentery and how is it treated?");
        let out = go(&s, &u, 300, "0.1");
        let l = out.to_lowercase();
        check("answers from excerpts", &out,
            l.contains("bacteri") && l.contains("antibiotic") && !out.contains(NOT_IN_LIBRARY));

        let (s, u) = ask_prompt(rich, "What is the population of Paris?");
        let out = go(&s, &u, 300, "0.1");
        check("refuses an off-topic question", &out, out.contains(NOT_IN_LIBRARY));

        for (target, text, want) in [
            ("English", "¿Dónde está el hospital más cercano? Mi amigo está herido.", ["hospital", "friend"]),
            ("English", "Ne buvez pas l'eau de la rivière avant de l'avoir fait bouillir.", ["drink", "boil"]),
            ("Spanish", "Boil the water for one minute before drinking it.", ["agua", "minuto"]),
            ("French", "Keep the wound clean and covered.", ["propre", "plaie"]),
        ] {
            let (s, u) = translate_prompt(target, text);
            let out = go(&s, &u, 200, "0");
            let l = out.to_lowercase();
            check(&format!("translate into {target}: {text}"), &out,
                want.iter().all(|w| l.contains(w)) && !out.contains("Text:") && !out.starts_with('"'));
        }
        assert!(fails.is_empty(), "failed: {fails:?}");
    }
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn next_generation() -> u64 {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
}

pub fn run(
    mut cmd: std::process::Command,
    model: &std::path::Path,
    template: &std::path::Path,
    system: &str,
    prompt: &str,
    max_tokens: u32,
    temp: &str,
    gen: u64,
    on_text: &mut dyn FnMut(&str),
) -> Result<String, String> {
    use std::io::Read;
    let prompt = prompt.trim();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .to_string();
    let mut child = cmd
        .arg("-m")
        .arg(model)
        .arg("--jinja")
        .arg("--chat-template-file")
        .arg(template)
        .args(["-cnv", "-st", "--no-warmup", "--simple-io", "-c", "8192"])
        .args(["--no-display-prompt", "--temp", temp, "-t", &threads])
        .args(["--top-k", "50", "--repeat-penalty", "1.1"])
        .arg("-n")
        .arg(max_tokens.to_string())
        .arg("-sys")
        .arg(system)
        .arg("-p")
        .arg(prompt)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the local model: {e}"))?;
    let mut stderr = child.stderr.take().ok_or("model stderr")?;
    let err_thread = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = stderr.read_to_end(&mut s);
        s
    });
    let mut stdout = child.stdout.take().ok_or("model stdout")?;
    let mut stream = Stream::new();
    let mut answer = String::new();
    let mut buf = [0u8; 512];
    let mut cancelled = false;
    loop {
        let n = stdout.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        if GENERATION.load(std::sync::atomic::Ordering::SeqCst) != gen {
            cancelled = true;
            let _ = child.kill();
            break;
        }
        let t = stream.feed(&buf[..n]);
        if !t.is_empty() {
            answer.push_str(&t);
            if let Some(v) = streaming_view(&answer) {
                on_text(&v);
            }
        }
    }
    answer.push_str(&stream.finish());
    if !answer.trim().is_empty() {
        on_text(&visible_answer(&answer));
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let err = err_thread.join().unwrap_or_default();
    if cancelled {
        return Err("cancelled".into());
    }
    if !status.success() && answer.trim().is_empty() {
        let tail: String = String::from_utf8_lossy(&err)
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .join(" / ");
        return Err(format!("the local model failed: {tail}"));
    }
    Ok(visible_answer(&answer))
}
