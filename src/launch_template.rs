use crate::config::Config;
use anyhow::{Result, anyhow, bail};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchTemplatePreset {
    pub id: String,
    pub label: String,
    pub session_prefix: String,
    pub command: String,
    pub window_name: Option<String>,
}

pub fn launch_template_presets(config: &Config) -> Vec<LaunchTemplatePreset> {
    let mut presets = vec![LaunchTemplatePreset {
        id: "pi".to_string(),
        label: "Pi Coding Agent".to_string(),
        session_prefix: "pi".to_string(),
        command: "pi".to_string(),
        window_name: Some("agent".to_string()),
    }];
    presets.extend(config.launch_templates.presets.iter().map(|preset| {
        LaunchTemplatePreset {
            id: preset.id.trim().to_string(),
            label: preset.label.trim().to_string(),
            session_prefix: preset.session_prefix.trim().to_string(),
            command: preset.command.trim().to_string(),
            window_name: preset
                .window_name
                .as_deref()
                .map(str::trim)
                .filter(|window_name| !window_name.is_empty())
                .map(str::to_string),
        }
    }));
    presets
}

pub fn launch_template_preset(config: &Config, id: &str) -> Result<LaunchTemplatePreset> {
    let id = id.trim();
    if id.is_empty() {
        bail!("launch template id must not be empty");
    }
    launch_template_presets(config)
        .into_iter()
        .find(|preset| preset.id == id)
        .ok_or_else(|| anyhow!("unknown launch template `{id}`"))
}

pub fn launch_session_name(preset: &LaunchTemplatePreset, suffix: &str) -> Result<String> {
    let suffix = suffix.trim();
    if suffix.is_empty() {
        bail!("launch template expects a session name");
    }
    if has_target_separator(suffix) {
        bail!("launch template session name must not contain `/` or `:`");
    }
    let session_name = format!("{}-{suffix}", preset.session_prefix);
    if has_target_separator(&session_name) {
        bail!("launch template session name must not contain `/` or `:`");
    }
    Ok(session_name)
}

pub fn launch_template_label(preset: &LaunchTemplatePreset) -> String {
    format!(
        "{} ({} -> {})",
        preset.label, preset.session_prefix, preset.command
    )
}

fn has_target_separator(value: &str) -> bool {
    value.contains('/') || value.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_pi_launch_template_is_first() {
        let config: Config = yaml_serde::from_str(
            r#"
launch_templates:
  presets:
    - id: custom
      label: Custom Agent
      session_prefix: custom
      command: custom-agent
"#,
        )
        .unwrap();

        let presets = launch_template_presets(&config);
        let ids: Vec<&str> = presets.iter().map(|preset| preset.id.as_str()).collect();

        assert_eq!(ids, vec!["pi", "custom"]);
    }

    #[test]
    fn launch_session_name_uses_prefix() {
        let preset = LaunchTemplatePreset {
            id: "agent".to_string(),
            label: "Agent".to_string(),
            session_prefix: "agent".to_string(),
            command: "pi".to_string(),
            window_name: None,
        };

        assert_eq!(
            launch_session_name(&preset, "implement-auth").unwrap(),
            "agent-implement-auth"
        );
    }

    #[test]
    fn launch_session_name_rejects_empty_suffix() {
        let preset = LaunchTemplatePreset {
            id: "agent".to_string(),
            label: "Agent".to_string(),
            session_prefix: "agent".to_string(),
            command: "pi".to_string(),
            window_name: None,
        };

        assert!(
            launch_session_name(&preset, " ")
                .unwrap_err()
                .to_string()
                .contains("expects a session name")
        );
    }

    #[test]
    fn launch_session_name_rejects_target_separators() {
        let preset = LaunchTemplatePreset {
            id: "agent".to_string(),
            label: "Agent".to_string(),
            session_prefix: "agent".to_string(),
            command: "pi".to_string(),
            window_name: None,
        };

        assert!(
            launch_session_name(&preset, "api/work")
                .unwrap_err()
                .to_string()
                .contains("must not contain")
        );
        assert!(
            launch_session_name(&preset, "api:0")
                .unwrap_err()
                .to_string()
                .contains("must not contain")
        );
    }
}
