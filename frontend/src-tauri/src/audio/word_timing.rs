use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordTiming {
    pub byte_start: usize,
    pub start: f64,
    pub end: f64,
}

pub(crate) fn shift_words(words: &mut [WordTiming], offset: f64) {
    for word in words {
        word.start = (word.start + offset).max(0.0);
        word.end = (word.end + offset).max(0.0);
    }
}

/// Detokenize before looking for words. A Russian letter may span several
/// Whisper tokens. SentencePiece continuations also belong to the same word.
pub(crate) fn words_from_pieces(
    text: &str,
    pieces: impl IntoIterator<Item = (impl AsRef<[u8]>, f64, f64)>,
    duration: f64,
    join_apostrophe: bool,
) -> Option<Vec<WordTiming>> {
    let mut raw = Vec::new();
    let mut times = Vec::new();
    for (piece, start, end) in pieces {
        let bytes = piece.as_ref();
        raw.extend_from_slice(bytes);
        times.resize(raw.len(), (start, end));
    }

    let mut decoded = Vec::with_capacity(raw.len());
    let mut offsets = Vec::with_capacity(raw.len());
    for (index, &byte) in raw.iter().enumerate() {
        if join_apostrophe && byte == b' ' && raw.get(index + 1) == Some(&b'\'') {
            continue;
        }
        decoded.push(byte);
        offsets.push(index);
    }
    let decoded = std::str::from_utf8(&decoded).ok()?;
    if decoded.trim() != text {
        return None;
    }
    let leading = decoded.len() - decoded.trim_start().len();

    let mut words = Vec::<WordTiming>::new();
    let mut opening = None;
    let mut position = 0;
    for chunk in text.split_inclusive(char::is_whitespace) {
        let chunk_start = position;
        position += chunk.len();
        let mut start = f64::INFINITY;
        let mut end = f64::NEG_INFINITY;
        let mut lexical = false;
        for (offset, character) in chunk.char_indices().filter(|(_, c)| c.is_alphanumeric()) {
            lexical = true;
            let first = leading + chunk_start + offset;
            for &index in &offsets[first..first + character.len_utf8()] {
                let (token_start, token_end) = times[index];
                if token_start >= 0.0 && token_end >= token_start {
                    start = start.min(token_start.min(duration));
                    end = end.max(token_end.min(duration));
                }
            }
        }
        if lexical {
            if !start.is_finite() || end < start {
                return None;
            }
            let byte_start = opening.take().unwrap_or(chunk_start);
            words.push(WordTiming {
                byte_start: if words.is_empty() { 0 } else { byte_start },
                start,
                end,
            });
        } else if chunk
            .trim()
            .chars()
            .next()
            .is_some_and(|c| matches!(c, '«' | '„' | '“' | '(' | '[' | '{' | '—' | '–'))
        {
            opening.get_or_insert(chunk_start);
        }
    }
    Some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn represents_split_utf8_and_standalone_russian_punctuation() {
        let yes = "Да".as_bytes();
        let pieces = [
            (" Вопрос?".as_bytes(), 0.0, 0.7),
            (" « ".as_bytes(), 0.7, 0.9),
            (&yes[..1], 1.0, 1.1),
            (&yes[1..], 1.1, 1.2),
            (" » — дальше.".as_bytes(), 1.5, 1.9),
        ];
        let text = "Вопрос? « Да » — дальше.";
        let words = words_from_pieces(text, pieces, 2.0, false).unwrap();
        assert_eq!(words.len(), 3);
        assert_eq!(&text[words[1].byte_start..words[2].byte_start], "« Да » ");
        assert_eq!((words[1].start, words[1].end), (1.0, 1.2));
    }
}
