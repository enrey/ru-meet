//! Per-section report generation for the built-in model.
//!
//! A 4B model asked to fill a whole template in one prompt writes every
//! section thinly. Asking for one section per request, with Russian
//! instructions placed after the transcript and checkable numeric limits,
//! roughly doubles the detail it keeps; `Template::per_section` opts a
//! template into this path. The prompts were tuned on Qwen 3.5 4B, in
//! Russian, so the caller only uses them when the report is written in
//! Russian.

use std::path::PathBuf;

use reqwest::Client;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::summary::llm_client::{generate_summary, LLMProvider};
use crate::summary::processor::{clean_llm_markdown_detailed, require_visible_markdown};
use crate::summary::templates::{Template, TemplateSection};

const SYSTEM_PROMPT: &str = "Ты — сильный бизнес-аналитик и редактор рабочих коммуникаций. Ты превращаешь сырую автоматическую транскрибацию рабочей встречи в деловой статус на русском языке.";

const SHARED_RULES: &str = "\
1. Используй только то, что есть в транскрибации. Не добавляй своих рекомендаций и выводов.
2. Имена, названия систем, доменов и цифры пиши, только если они однозначно понятны из транскрибации. Если сомневаешься — опусти деталь или сформулируй нейтрально.
3. Пропускай бытовые разговоры, шутки, личные истории и отступления, не связанные с работой.
4. Различай: уже сделано / в процессе / планируется / только обсуждалось как идея. Не выдавай идею за решение, а план — за сделанное.
5. Пиши деловым русским языком: коротко, конкретно, без канцелярита и без пересказа «кто что сказал».";

const SPEAKER_RULE: &str = "6. Метки «Спикер N» нужны только чтобы понять, кто что сделал и кто что взял на себя: «я сделал» в реплике Спикера 1 значит, что сделал Спикер 1. Людей, о которых говорят в третьем лице, не путай с говорящими. В тексте раздела метки «Спикер N» не упоминай.";

const SPEAKER_PREFIX: &str = "Спикер ";

/// Rewrite the diarization labels the transcript arrives with ("Speaker 3:",
/// "Others:") into the Russian ones the prompts talk about. Names the user
/// gave speakers are left as they are.
fn localize_speaker_labels(transcript: &str) -> String {
    transcript
        .lines()
        .map(|line| {
            if let Some(rest) = line.strip_prefix("Others:") {
                return format!("Другие:{rest}");
            }
            match line.strip_prefix("Speaker ") {
                Some(rest) if speaker_number_len(rest) > 0 => format!("{SPEAKER_PREFIX}{rest}"),
                _ => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Length of the "N:" speaker number at the start of `rest`, without the
/// colon; 0 when `rest` does not start with one.
fn speaker_number_len(rest: &str) -> usize {
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && rest[digits..].starts_with(':') {
        digits
    } else {
        0
    }
}

fn has_speaker_labels(transcript: &str) -> bool {
    transcript.lines().any(|line| {
        line.strip_prefix(SPEAKER_PREFIX)
            .is_some_and(|rest| speaker_number_len(rest) > 0)
    })
}

/// Shared beginning of every request: transcript first, instructions after
/// it, which a small model follows more reliably than the reverse.
fn build_head(transcript: &str, custom_prompt: &str) -> String {
    let mut head = format!(
        "<транскрибация>\n{transcript}\n</транскрибация>\n\nВыше — автоматическая транскрибация рабочей встречи. В ней есть ошибки распознавания, обрывки фраз, шутки и бытовые разговоры."
    );
    if has_speaker_labels(transcript) {
        head.push_str(" Реплики размечены метками говорящих: «Спикер 1», «Спикер 2».");
    }
    head.push_str("\n\n");
    if !custom_prompt.trim().is_empty() {
        head.push_str(&format!(
            "Контекст от пользователя (справка, не часть транскрибации):\n<контекст>\n{}\n</контекст>\n\n",
            custom_prompt.trim()
        ));
    }
    head
}

fn shared_rules(transcript: &str) -> String {
    if has_speaker_labels(transcript) {
        format!("{SHARED_RULES}\n{SPEAKER_RULE}")
    } else {
        SHARED_RULES.to_string()
    }
}

fn title_prompt(head: &str) -> String {
    format!(
        "{head}Задача: придумай короткое деловое название встречи (до 10 слов), отражающее главные темы.\n\nВыведи только название одной строкой, без кавычек, без «#» и без пояснений."
    )
}

/// Shape of the section for templates that give no explicit `layout`.
fn layout_for(section: &TemplateSection) -> String {
    if let Some(layout) = &section.layout {
        return layout.clone();
    }
    let mut layout = match section.format.as_str() {
        "paragraph" => "Связный текст без списков.",
        "string" => "Одна строка.",
        _ => "Маркированный список.",
    }
    .to_string();
    if let Some(item) = section.item_format.as_ref().or(section.example_item_format.as_ref()) {
        layout.push_str(&format!(" Каждый пункт в формате: {item}."));
    }
    layout
}

fn section_prompt(
    head: &str,
    rules: &str,
    section: &TemplateSection,
    finished: &[(String, String)],
) -> String {
    let earlier = section
        .builds_on
        .as_ref()
        .and_then(|name| finished.iter().find(|(title, _)| title == name))
        .map(|(name, text)| {
            format!("Уже готовый раздел «{name}»:\n{text}\n\nНе повторяй его пункты и не противоречь им.\n\n")
        })
        .unwrap_or_default();
    format!(
        "{head}{earlier}Задача: напиши раздел «{title}» итогового статуса встречи.\n\nОбщие правила:\n{rules}\n\nПравила раздела «{title}»:\n{instruction}\n\nФормат:\n{layout}\n\nВыведи только этот раздел: первая строка — «**{title}**», затем содержание. Без рассуждений, без названия встречи и без других разделов.",
        title = section.title,
        instruction = section.instruction,
        layout = layout_for(section),
    )
}

/// Drop a leading "Спикер N:" the model copied from the transcript into a
/// line or bullet. Labels inside a sentence are left alone: removing them
/// would break the sentence.
fn strip_speaker_labels(markdown: &str) -> String {
    markdown
        .lines()
        .map(|line| {
            let indent_len = line.len() - line.trim_start().len();
            let (indent, rest) = line.split_at(indent_len);
            let (bullet, body) = ["- ", "* "]
                .iter()
                .find_map(|bullet| rest.strip_prefix(bullet).map(|body| (*bullet, body.trim_start())))
                .unwrap_or(("", rest));
            let body = body
                .strip_prefix(SPEAKER_PREFIX)
                .and_then(|after| {
                    let digits = speaker_number_len(after);
                    (digits > 0).then(|| after[digits + 1..].trim_start())
                })
                .unwrap_or(body);
            format!("{indent}{bullet}{body}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn clean_section_body(title: &str, markdown: &str) -> String {
    let body = markdown.trim();
    let body = body
        .strip_prefix(&format!("**{title}**"))
        .unwrap_or(body)
        .trim();
    strip_speaker_labels(body)
}

fn clean_title(markdown: &str) -> String {
    markdown
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_matches(['"', '«', '»'])
        .trim()
        .to_string()
}

fn assemble_report(title: &str, sections: &[(String, String)]) -> String {
    let mut report = if title.is_empty() {
        String::new()
    } else {
        format!("# {title}\n\n")
    };
    for (name, body) in sections {
        report.push_str(&format!("**{name}**\n\n{body}\n\n"));
    }
    report.trim_end().to_string()
}

/// One built-in model request; returns the visible markdown and whether
/// reasoning had to be stripped from it.
async fn generate_one(
    client: &Client,
    model_name: &str,
    app_data_dir: Option<&PathBuf>,
    cancellation_token: Option<&CancellationToken>,
    stage: String,
    user_prompt: String,
) -> Result<(String, bool), String> {
    if cancellation_token.is_some_and(CancellationToken::is_cancelled) {
        return Err("Summary generation was cancelled".to_string());
    }
    let completion = generate_summary(
        client,
        &LLMProvider::BuiltInAI,
        model_name,
        "",
        SYSTEM_PROMPT,
        &user_prompt,
        None,
        None,
        None,
        None,
        None,
        app_data_dir,
        cancellation_token,
    )
    .await
    .map_err(|error| format!("{stage} failed: {error}"))?;
    let cleaned = clean_llm_markdown_detailed(&completion.content);
    require_visible_markdown(&stage, &cleaned)?;
    Ok((
        cleaned.markdown,
        completion.reasoning_stripped || cleaned.reasoning_stripped,
    ))
}

pub(crate) struct SectionReport {
    pub markdown: String,
    pub reasoning_stripped: bool,
}

/// Generate the report with one built-in model request for the title and one
/// per template section, in template order.
pub(crate) async fn generate_report(
    client: &Client,
    model_name: &str,
    app_data_dir: Option<&PathBuf>,
    transcript: &str,
    custom_prompt: &str,
    template: &Template,
    cancellation_token: Option<&CancellationToken>,
) -> Result<SectionReport, String> {
    let transcript = localize_speaker_labels(transcript);
    let head = build_head(&transcript, custom_prompt);
    let rules = shared_rules(&transcript);
    let mut reasoning_stripped = false;

    let ask = |stage: String, user_prompt: String| {
        generate_one(client, model_name, app_data_dir, cancellation_token, stage, user_prompt)
    };

    let (title, stripped) = ask("Summary title".to_string(), title_prompt(&head)).await?;
    reasoning_stripped |= stripped;
    let title = clean_title(&title);

    let mut finished: Vec<(String, String)> = Vec::with_capacity(template.sections.len());
    for (index, section) in template.sections.iter().enumerate() {
        info!(
            "Generating summary section {}/{}: {}",
            index + 1,
            template.sections.len(),
            section.title
        );
        let prompt = section_prompt(&head, &rules, section, &finished);
        let (markdown, stripped) =
            ask(format!("Summary section '{}'", section.title), prompt).await?;
        reasoning_stripped |= stripped;
        finished.push((section.title.clone(), clean_section_body(&section.title, &markdown)));
    }

    Ok(SectionReport {
        markdown: assemble_report(&title, &finished),
        reasoning_stripped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(title: &str, builds_on: Option<&str>) -> TemplateSection {
        TemplateSection {
            title: title.to_string(),
            instruction: format!("Правила для {title}."),
            format: "list".to_string(),
            item_format: None,
            example_item_format: None,
            layout: None,
            builds_on: builds_on.map(str::to_string),
        }
    }

    #[test]
    fn diarization_labels_become_russian_and_names_stay() {
        let transcript = "Speaker 1: привет\nOthers: да\nАнна: ок\nSpeaker one: нет";
        assert_eq!(
            localize_speaker_labels(transcript),
            "Спикер 1: привет\nДругие: да\nАнна: ок\nSpeaker one: нет"
        );
    }

    #[test]
    fn speaker_rule_only_when_transcript_is_labelled() {
        let labelled = localize_speaker_labels("Speaker 2: текст");
        assert!(shared_rules(&labelled).contains("Спикер N"));
        assert!(build_head(&labelled, "").contains("метками говорящих"));

        let plain = "просто текст";
        assert!(!shared_rules(plain).contains("Спикер N"));
        assert!(!build_head(plain, "").contains("метками говорящих"));
    }

    #[test]
    fn instructions_follow_the_transcript() {
        let head = build_head("Спикер 1: текст", "проект Альфа");
        let prompt = section_prompt(&head, SHARED_RULES, &section("Риски", None), &[]);
        let transcript_end = prompt.find("</транскрибация>").unwrap();
        assert!(prompt.find("Задача:").unwrap() > transcript_end);
        assert!(prompt.find("проект Альфа").unwrap() > transcript_end);
    }

    #[test]
    fn section_sees_the_section_it_builds_on() {
        let finished = vec![("Договорённости".to_string(), "- Почистить бэкапы".to_string())];
        let prompt = section_prompt(
            "",
            SHARED_RULES,
            &section("Открытые вопросы", Some("Договорённости")),
            &finished,
        );
        assert!(prompt.contains("Уже готовый раздел «Договорённости»:\n- Почистить бэкапы"));
        let prompt = section_prompt("", SHARED_RULES, &section("Риски", None), &finished);
        assert!(!prompt.contains("Почистить бэкапы"));
    }

    #[test]
    fn leading_speaker_labels_are_removed_from_bullets() {
        let markdown = "- Спикер 1: почистил бэкапы\n    *   Спикер 12: проверит VPN\nСпикер 2: итог\n- Спикер 1 изучит вопрос";
        assert_eq!(
            strip_speaker_labels(markdown),
            "- почистил бэкапы\n    * проверит VPN\nитог\n- Спикер 1 изучит вопрос"
        );
    }

    #[test]
    fn report_has_title_and_bold_section_headings() {
        let body = clean_section_body("Резюме", "**Резюме**\n\nТекст.");
        let report = assemble_report(
            &clean_title("# «Статус инфраструктуры»\n"),
            &[("Резюме".to_string(), body)],
        );
        assert_eq!(report, "# Статус инфраструктуры\n\n**Резюме**\n\nТекст.");
    }

    #[test]
    fn layout_falls_back_to_section_format() {
        let mut list = section("Задачи", None);
        list.item_format = Some("Что — Кто — Срок".to_string());
        assert_eq!(
            layout_for(&list),
            "Маркированный список. Каждый пункт в формате: Что — Кто — Срок."
        );
        list.layout = Some("2–8 пунктов.".to_string());
        assert_eq!(layout_for(&list), "2–8 пунктов.");
    }
}
