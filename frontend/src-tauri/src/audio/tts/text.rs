//! Turning a summary into utterances the TTS model can speak.
//!
//! Two steps: strip the markdown structure, then cut the plain text into
//! sentence-sized chunks. Nothing else is rewritten - the model is a language
//! model and reads ordinary text, including digits, punctuation and latin
//! words, better than any normalizer would rewrite them for it.

/// Hard ceiling for one utterance. A sentence rarely reaches it; one that does
/// gets cut at a comma rather than handed to the model whole, because long
/// inputs make VITS drift.
const MAX_CHUNK_CHARS: usize = 240;

/// One entry per surviving source line: a heading, a bullet or a paragraph.
/// Kept apart so a chunk never spans two of them.
fn strip_markdown(markdown: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut inside_code_block = false;

    for raw_line in markdown.lines() {
        let line = raw_line.trim();

        if line.starts_with("```") {
            inside_code_block = !inside_code_block;
            continue;
        }
        if inside_code_block {
            continue;
        }
        // Tables and horizontal rules carry no speakable content.
        let is_rule = line.len() >= 3 && line.chars().all(|c| matches!(c, '-' | '*' | '_'));
        if line.starts_with('|') || is_rule {
            continue;
        }
        if line.is_empty() {
            continue;
        }

        let mut text = strip_leading_markers(line);
        text = strip_links(&text);
        text = text.replace("**", "").replace("__", "");
        text = text.replace(['*', '`', '#', '>', '[', ']'], "");
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }

        // A heading or a bullet is its own utterance: end it with a full stop
        // so the model pauses instead of running into the next line.
        if text.ends_with(['.', '!', '?', ':', ';']) {
            lines.push(text);
        } else {
            lines.push(format!("{text}."));
        }
    }

    lines
}

fn strip_leading_markers(line: &str) -> String {
    let mut rest = line.trim_start();
    loop {
        let trimmed = rest
            .trim_start_matches(['#', '>', '-', '*', '+'])
            .trim_start();
        let trimmed = match trimmed.split_once(". ") {
            // "1. Пункт" - a numbered bullet, not a sentence.
            Some((prefix, tail))
                if !prefix.is_empty() && prefix.chars().all(|c| c.is_ascii_digit()) =>
            {
                tail
            }
            _ => trimmed,
        };
        if trimmed == rest {
            return rest.to_string();
        }
        rest = trimmed;
    }
}

fn strip_links(line: &str) -> String {
    // `[label](url)` keeps the label; the url is never speakable.
    let mut output = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find("](") {
        let (label_part, tail) = rest.split_at(open);
        output.push_str(label_part);
        match tail[2..].find(')') {
            Some(close) => rest = &tail[2 + close + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    output.push_str(rest);
    output
}

/// Whether a chunk carries anything worth speaking.
fn has_letters(text: &str) -> bool {
    text.chars().any(char::is_alphabetic)
}

fn collapse_spaces(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut last_was_space = false;
    for character in text.chars() {
        if character == ' ' {
            if !last_was_space {
                output.push(' ');
            }
            last_was_space = true;
        } else {
            output.push(character);
            last_was_space = false;
        }
    }
    output.trim().to_string()
}

/// Cut one paragraph at its sentence ends, keeping the punctuation.
fn sentences_of(paragraph: &str) -> Vec<String> {
    let mut sentences: Vec<String> = Vec::new();
    let mut current = String::new();
    for character in paragraph.chars() {
        current.push(character);
        if matches!(character, '.' | '!' | '?') {
            let sentence = current.trim().to_string();
            if !sentence.is_empty() {
                sentences.push(sentence);
            }
            current = String::new();
        }
    }
    let tail = current.trim().to_string();
    if !tail.is_empty() {
        sentences.push(tail);
    }
    sentences
}

/// A sentence past the ceiling is cut at the last comma-like break before it,
/// and only split between words when it has no such break at all.
fn split_long_sentence(sentence: &str) -> Vec<String> {
    if sentence.chars().count() <= MAX_CHUNK_CHARS {
        return vec![sentence.to_string()];
    }

    let characters: Vec<char> = sentence.chars().collect();
    let head: String = characters[..MAX_CHUNK_CHARS].iter().collect();
    let cut = head
        .rfind([',', ';', ':', '-'])
        .map(|index| index + 1)
        .or_else(|| head.rfind(' '))
        .unwrap_or(head.len());

    let (first, rest) = head.split_at(cut);
    let remainder: String = rest
        .chars()
        .chain(characters[MAX_CHUNK_CHARS..].iter().copied())
        .collect();

    let mut parts = vec![first.trim().to_string()];
    parts.extend(split_long_sentence(remainder.trim()));
    parts.retain(|part| !part.is_empty());
    parts
}

/// Full pipeline: summary markdown in, speakable chunks out, each tagged with
/// the index of the paragraph (heading, bullet, ...) it came from.
///
/// A chunk is one sentence. Chunks never span two paragraphs, so a heading or a
/// bullet always ends the utterance, and the pause between chunks falls where
/// the text itself pauses.
pub fn summary_to_chunks(markdown: &str) -> Vec<(usize, String)> {
    let mut chunks: Vec<(usize, String)> = Vec::new();

    for (index, paragraph) in strip_markdown(markdown).iter().enumerate() {
        let speakable = collapse_spaces(paragraph);
        if !has_letters(&speakable) {
            continue;
        }

        for sentence in sentences_of(&speakable) {
            chunks.extend(
                split_long_sentence(&sentence)
                    .into_iter()
                    .filter(|chunk| has_letters(chunk))
                    .map(|chunk| (index, chunk)),
            );
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(markdown: &str) -> Vec<String> {
        summary_to_chunks(markdown)
            .into_iter()
            .map(|(_, chunk)| chunk)
            .collect()
    }

    #[test]
    fn ordinary_text_reaches_the_model_unchanged() {
        let text = texts("- Выполнено 5 задач из 12, рост 30%.").join(" ");
        assert_eq!(text, "Выполнено 5 задач из 12, рост 30%.");
    }

    #[test]
    fn markdown_structure_is_removed() {
        let text = texts("## Решения\n\n- **Первое** решение\n- Второе решение").join(" ");
        assert!(!text.contains('#') && !text.contains('*'), "{text}");
        assert!(text.contains("Решения."), "{text}");
    }

    #[test]
    fn chunks_stay_within_the_model_limit() {
        let long = "Обсудили план и распределили задачи между участниками, ".repeat(20);
        for chunk in texts(&long) {
            assert!(chunk.chars().count() <= MAX_CHUNK_CHARS, "{chunk}");
        }
    }

    #[test]
    fn a_chunk_is_one_sentence_and_never_spans_paragraphs() {
        let chunks = texts("## Решения\n\n- Первое решение. Второе решение.\n- Третье решение.");
        assert_eq!(
            chunks,
            vec![
                "Решения.",
                "Первое решение.",
                "Второе решение.",
                "Третье решение."
            ]
        );
    }

    #[test]
    fn chunks_remember_their_paragraph() {
        assert_eq!(
            summary_to_chunks(
                "Резюме

---

Первое. Второе."
            ),
            vec![
                (0, "Резюме.".to_string()),
                (1, "Первое.".to_string()),
                (1, "Второе.".to_string()),
            ]
        );
    }
}
