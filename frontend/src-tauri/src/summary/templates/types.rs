use serde::{Deserialize, Serialize};

/// Represents a single section in a meeting template
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateSection {
    /// Section title (e.g., "Summary", "Action Items")
    pub title: String,

    /// Instruction for the LLM on what to extract/include
    pub instruction: String,

    /// Format type: "paragraph", "list", or "string"
    pub format: String,

    /// Optional markdown formatting hint for list items (e.g., table structure)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_format: Option<String>,

    /// Alternative formatting hint
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example_item_format: Option<String>,

    /// Concrete shape and length limits, e.g. "2–8 пунктов, каждый не длиннее
    /// 25 слов". Small local models follow numbers far better than adjectives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,

    /// Title of an earlier section whose finished text is shown to this one
    /// in per-section generation, so it neither repeats nor contradicts it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builds_on: Option<String>,
}

/// Represents a complete meeting template
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    /// Template display name
    pub name: String,

    /// Brief description of the template's purpose
    pub description: String,

    /// List of sections in the template
    pub sections: Vec<TemplateSection>,

    /// Generate the built-in model's report one section per request instead
    /// of one prompt for the whole template (see `summary::section_pass`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub per_section: bool,
}

impl Template {
    /// Validates the template structure
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("Template name cannot be empty".to_string());
        }

        if self.description.is_empty() {
            return Err("Template description cannot be empty".to_string());
        }

        if self.sections.is_empty() {
            return Err("Template must have at least one section".to_string());
        }

        for (i, section) in self.sections.iter().enumerate() {
            if section.title.is_empty() {
                return Err(format!("Section {} has empty title", i));
            }

            if section.instruction.is_empty() {
                return Err(format!("Section '{}' has empty instruction", section.title));
            }

            match section.format.as_str() {
                "paragraph" | "list" | "string" => {},
                other => return Err(format!(
                    "Section '{}' has invalid format '{}'. Must be 'paragraph', 'list', or 'string'",
                    section.title, other
                )),
            }

            if let Some(earlier) = &section.builds_on {
                if !self.sections[..i].iter().any(|s| &s.title == earlier) {
                    return Err(format!(
                        "Section '{}' builds on '{}', which is not an earlier section",
                        section.title, earlier
                    ));
                }
            }
        }

        Ok(())
    }

    /// Generates a clean markdown template structure
    pub fn to_markdown_structure(&self) -> String {
        let mut markdown = String::from("# <Add Title here>\n\n");

        for section in &self.sections {
            markdown.push_str(&format!("**{}**\n\n", section.title));
        }

        markdown
    }

    /// Generates section-specific instructions for the LLM
    pub fn to_section_instructions(&self) -> String {
        let mut instructions = String::from(
            "- **For the main title (`# [AI-Generated Title]`):** Analyze the entire transcript and create a concise, descriptive title for the meeting.\n"
        );

        for section in &self.sections {
            instructions.push_str(&format!(
                "- **For the '{}' section:** {}.\n",
                section.title, section.instruction
            ));

            // Add item format instructions if present
            let item_format = section
                .item_format
                .as_ref()
                .or(section.example_item_format.as_ref());

            if let Some(format) = item_format {
                instructions.push_str(&format!(
                    "  - Items in this section should follow the format: `{}`.\n",
                    format
                ));
            }
        }

        instructions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_template() {
        let template = Template {
            name: "Test Template".to_string(),
            description: "A test template".to_string(),
            sections: vec![TemplateSection {
                title: "Summary".to_string(),
                instruction: "Provide a summary".to_string(),
                format: "paragraph".to_string(),
                item_format: None,
                example_item_format: None,
                layout: None,
                builds_on: None,
            }],
            per_section: false,
        };

        assert!(template.validate().is_ok());
    }

    #[test]
    fn test_validate_empty_name() {
        let template = Template {
            name: "".to_string(),
            description: "A test template".to_string(),
            sections: vec![],
            per_section: false,
        };

        assert!(template.validate().is_err());
    }

    #[test]
    fn test_validate_invalid_format() {
        let template = Template {
            name: "Test".to_string(),
            description: "Test".to_string(),
            sections: vec![TemplateSection {
                title: "Test".to_string(),
                instruction: "Test".to_string(),
                format: "invalid".to_string(),
                item_format: None,
                example_item_format: None,
                layout: None,
                builds_on: None,
            }],
            per_section: false,
        };

        assert!(template.validate().is_err());
    }
}
