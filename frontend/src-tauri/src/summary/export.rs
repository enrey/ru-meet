use anyhow::{bail, Context, Result};
use pulldown_cmark::{html, Event, Options, Parser};
use serde_json::Value;
use sqlx::SqlitePool;
use std::path::Path;

const HTML_FILE_NAME: &str = "summary.html";
const MD5_FILE_NAME: &str = "summary.md5";

pub(crate) async fn write_summary_files_for_meeting(
    pool: &SqlitePool,
    meeting_id: &str,
    summary: &Value,
) -> Result<bool> {
    let meeting = crate::database::repositories::meeting::MeetingsRepository::get_meeting_metadata(
        pool, meeting_id,
    )
    .await
    .with_context(|| format!("Failed to load meeting metadata for {meeting_id}"))?
    .with_context(|| format!("Meeting not found: {meeting_id}"))?;

    let Some(folder_path) = meeting.folder_path.filter(|path| !path.trim().is_empty()) else {
        return Ok(false);
    };
    write_summary_files(Path::new(&folder_path), &meeting.title, summary)?;
    Ok(true)
}

/// Persist a browser-readable summary and a checksum sidecar in the meeting folder.
///
/// `summary.md5` contains the lowercase MD5 digest of the exact `summary.html`
/// bytes followed by a newline, matching the usual single-digest sidecar format.
pub(crate) fn write_summary_files(
    meeting_folder: &Path,
    meeting_title: &str,
    summary: &Value,
) -> Result<()> {
    if !meeting_folder.is_dir() {
        bail!(
            "Meeting folder does not exist or is not a directory: {}",
            meeting_folder.display()
        );
    }

    let markdown = summary_to_markdown(summary)
        .filter(|markdown| !markdown.trim().is_empty())
        .context("Summary contains no content that can be exported")?;
    let document = render_html_document(meeting_title, &markdown);
    let digest = format!("{:x}\n", md5::compute(document.as_bytes()));

    std::fs::write(meeting_folder.join(HTML_FILE_NAME), document).with_context(|| {
        format!(
            "Failed to write {} in {}",
            HTML_FILE_NAME,
            meeting_folder.display()
        )
    })?;
    std::fs::write(meeting_folder.join(MD5_FILE_NAME), digest).with_context(|| {
        format!(
            "Failed to write {} in {}",
            MD5_FILE_NAME,
            meeting_folder.display()
        )
    })?;

    Ok(())
}

fn summary_to_markdown(summary: &Value) -> Option<String> {
    let object = summary.as_object()?;
    if let Some(markdown) = object.get("markdown").and_then(Value::as_str) {
        if !markdown.trim().is_empty() {
            return Some(markdown.to_string());
        }
    }

    if let Some(blocks) = object.get("summary_json").and_then(Value::as_array) {
        let markdown = blocks
            .iter()
            .filter_map(block_to_markdown)
            .collect::<Vec<_>>()
            .join("\n\n");
        if !markdown.trim().is_empty() {
            return Some(markdown);
        }
    }

    // Older summaries are stored as named sections containing `blocks` with
    // textual `content`. Keep those meetings exportable as well.
    let mut sections = Vec::new();
    for (key, section) in object {
        if matches!(key.as_str(), "MeetingName" | "_section_order") {
            continue;
        }
        let Some(blocks) = section.get("blocks").and_then(Value::as_array) else {
            continue;
        };
        let contents = blocks
            .iter()
            .filter_map(|block| block.get("content").and_then(Value::as_str))
            .map(str::trim)
            .filter(|content| !content.is_empty())
            .collect::<Vec<_>>();
        if !contents.is_empty() {
            sections.push(format!("## {}\n\n{}", key, contents.join("\n\n")));
        }
    }

    (!sections.is_empty()).then(|| sections.join("\n\n"))
}

fn block_to_markdown(block: &Value) -> Option<String> {
    let text = collect_text(block.get("content")?).trim().to_string();
    if text.is_empty() {
        return None;
    }

    let block_type = block
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("paragraph");
    let rendered = match block_type {
        "heading" => {
            let level = block
                .pointer("/props/level")
                .and_then(Value::as_u64)
                .unwrap_or(2)
                .clamp(1, 6);
            format!("{} {}", "#".repeat(level as usize), text)
        }
        "bulletListItem" => format!("- {}", text),
        "numberedListItem" => format!("1. {}", text),
        "checkListItem" => {
            let checked = block
                .pointer("/props/checked")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            format!("- [{}] {}", if checked { "x" } else { " " }, text)
        }
        _ => text,
    };
    Some(rendered)
}

fn collect_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(collect_text).collect::<Vec<_>>().join(""),
        Value::Object(object) => object
            .get("text")
            .map(collect_text)
            .or_else(|| object.get("content").map(collect_text))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn render_html_document(meeting_title: &str, markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, Options::all()).map(|event| match event {
        // Summaries can contain model-produced markup. Display it as text instead
        // of allowing active HTML in the exported local document.
        Event::Html(raw) | Event::InlineHtml(raw) => Event::Text(raw),
        other => other,
    });
    let mut body = String::new();
    html::push_html(&mut body, parser);

    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>body{{max-width:900px;margin:40px auto;padding:0 24px;color:#1f2937;background:#fff;font:16px/1.65 system-ui,-apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif}}h1,h2,h3,h4,h5,h6{{line-height:1.25;color:#111827}}pre,code{{font-family:ui-monospace,SFMono-Regular,Consolas,monospace}}pre{{overflow:auto;padding:16px;background:#f3f4f6;border-radius:8px}}blockquote{{margin-left:0;padding-left:16px;border-left:4px solid #d1d5db;color:#4b5563}}table{{border-collapse:collapse}}th,td{{padding:8px 12px;border:1px solid #d1d5db}}a{{color:#2563eb}}</style>\n</head>\n<body>\n<main>\n<h1>{}</h1>\n{}\n</main>\n</body>\n</html>\n",
        escape_html(meeting_title),
        escape_html(meeting_title),
        body
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn writes_html_and_checksum_for_markdown_summary() {
        let directory = tempfile::tempdir().unwrap();
        write_summary_files(
            directory.path(),
            "Planning & Review",
            &json!({"markdown": "## Decision\n\nShip it."}),
        )
        .unwrap();

        let html = std::fs::read_to_string(directory.path().join(HTML_FILE_NAME)).unwrap();
        let checksum = std::fs::read_to_string(directory.path().join(MD5_FILE_NAME)).unwrap();
        assert!(html.contains("Planning &amp; Review"));
        assert!(html.contains("<h2>Decision</h2>"));
        assert_eq!(checksum, format!("{:x}\n", md5::compute(html.as_bytes())));
    }

    #[test]
    fn escapes_raw_html_from_summary() {
        let html = render_html_document("Meeting", "<script>alert('x')</script>");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn converts_blocknote_fallback_to_html_source_markdown() {
        let summary = json!({
            "summary_json": [{
                "type": "heading",
                "props": {"level": 2},
                "content": [{"type": "text", "text": "Actions"}]
            }]
        });
        assert_eq!(summary_to_markdown(&summary).as_deref(), Some("## Actions"));
    }
}
