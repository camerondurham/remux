use crate::config::Config;
use anyhow::{Result, bail};
use chrono::Utc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionTemplatePreset {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) prefix: SessionTemplatePrefix,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SessionTemplatePrefix {
    Date,
    Literal(String),
}

pub(super) fn session_template_presets(config: &Config) -> Vec<SessionTemplatePreset> {
    let mut presets = vec![
        SessionTemplatePreset {
            id: "date".to_string(),
            label: "Date".to_string(),
            prefix: SessionTemplatePrefix::Date,
        },
        literal_preset("work", "Work"),
        literal_preset("fix", "Fix"),
        literal_preset("spike", "Spike"),
    ];
    presets.extend(
        config
            .session_templates
            .presets
            .iter()
            .map(|preset| SessionTemplatePreset {
                id: preset.id.trim().to_string(),
                label: preset.label.trim().to_string(),
                prefix: SessionTemplatePrefix::Literal(preset.prefix.trim().to_string()),
            }),
    );
    presets
}

pub(super) fn templated_session_name(
    preset: &SessionTemplatePreset,
    suffix: &str,
) -> Result<String> {
    let suffix = suffix.trim();
    if suffix.is_empty() {
        bail!("template session expects a name");
    }
    if has_target_separator(suffix) {
        bail!("template session name must not contain `/` or `:`");
    }
    let prefix = match &preset.prefix {
        SessionTemplatePrefix::Date => Utc::now().format("%Y-%m-%d").to_string(),
        SessionTemplatePrefix::Literal(prefix) => prefix.clone(),
    };
    let session_name = format!("{prefix}-{suffix}");
    if has_target_separator(&session_name) {
        bail!("template session name must not contain `/` or `:`");
    }
    Ok(session_name)
}

pub(super) fn template_preset_label(preset: &SessionTemplatePreset) -> String {
    format!("{} ({})", preset.label, template_prefix_preview(preset))
}

pub(super) fn template_prefix_preview(preset: &SessionTemplatePreset) -> String {
    match &preset.prefix {
        SessionTemplatePrefix::Date => Utc::now().format("%Y-%m-%d").to_string(),
        SessionTemplatePrefix::Literal(prefix) => prefix.clone(),
    }
}

fn literal_preset(id: &str, label: &str) -> SessionTemplatePreset {
    SessionTemplatePreset {
        id: id.to_string(),
        label: label.to_string(),
        prefix: SessionTemplatePrefix::Literal(id.to_string()),
    }
}

fn has_target_separator(value: &str) -> bool {
    value.contains('/') || value.contains(':')
}
