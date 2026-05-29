use super::*;
use crate::config::{IndexedWatch, TmuxTarget, Watch, WatchMatchConfig};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[test]
fn command_and_cwd_watch_survives_coordinate_drift() {
    let watch = watch(
        "rpi2-kiro",
        WatchMatchConfig {
            command: Some("kiro-cli".to_string()),
            cwd: Some("/home/cam".to_string()),
            cwd_prefix: None,
            tmux: None,
        },
    );
    let panes = vec![pane("rpi2/0:3.2", "0", "3", "2", "kiro-cli", "/home/cam")];

    let (resolutions, claimed) = resolve_watch_matches(&[watch], &panes);

    assert_eq!(resolutions[0].status, MatchStatus::Matched);
    assert_eq!(resolutions[0].pane_index, Some(0));
    assert!(claimed.contains("rpi2/0:3.2"));
}

#[test]
fn ambiguous_watch_does_not_claim_candidates() {
    let watch = watch(
        "node-agent",
        WatchMatchConfig {
            command: Some("node".to_string()),
            cwd: None,
            cwd_prefix: Some("/home/cam/openclaw".to_string()),
            tmux: None,
        },
    );
    let panes = vec![
        pane(
            "rpi2/work:0.0",
            "work",
            "0",
            "0",
            "node",
            "/home/cam/openclaw",
        ),
        pane(
            "rpi2/work:0.1",
            "work",
            "0",
            "1",
            "node",
            "/home/cam/openclaw/src",
        ),
    ];

    let (resolutions, claimed) = resolve_watch_matches(&[watch], &panes);

    assert_eq!(resolutions[0].status, MatchStatus::Ambiguous);
    assert_eq!(resolutions[0].candidate_targets.len(), 2);
    assert!(claimed.is_empty());
}

#[test]
fn later_watch_is_shadowed_by_earlier_claim() {
    let first = watch(
        "first",
        WatchMatchConfig {
            command: Some("bash".to_string()),
            cwd: Some("/tmp".to_string()),
            cwd_prefix: None,
            tmux: None,
        },
    );
    let second = watch(
        "second",
        WatchMatchConfig {
            command: None,
            cwd: None,
            cwd_prefix: None,
            tmux: Some(TmuxTarget {
                session: "scratch".to_string(),
                window: Some(1),
                pane: Some(0),
            }),
        },
    );
    let panes = vec![pane(
        "local/scratch:1.0",
        "scratch",
        "1",
        "0",
        "bash",
        "/tmp",
    )];

    let (resolutions, claimed) = resolve_watch_matches(&[first, second], &panes);

    assert_eq!(resolutions[0].status, MatchStatus::Matched);
    assert_eq!(resolutions[1].status, MatchStatus::Shadowed);
    assert_eq!(resolutions[1].shadowed_by.as_deref(), Some("first"));
    assert!(claimed.contains("local/scratch:1.0"));
}

#[test]
fn infers_pi_agent_from_tmux_title_when_command_is_node() {
    let mut pane = pane("local/myservice:2.0", "myservice", "2", "0", "node", "/tmp");
    pane.pane_title = Some("\u{03c0} - work - Read the TaskPacket".to_string());

    assert_eq!(
        infer_coding_agent(
            &pane.command,
            pane.window_name.as_deref(),
            pane.pane_title.as_deref()
        ),
        Some("pi")
    );
}

#[test]
fn does_not_treat_plain_pi_host_title_as_agent() {
    assert_eq!(infer_coding_agent("node", None, Some("pi")), None);
}

fn watch(id: &str, matcher: WatchMatchConfig) -> IndexedWatch {
    IndexedWatch {
        index: 0,
        watch: Watch {
            id: id.to_string(),
            host: "rpi2".to_string(),
            matcher,
            repo: None,
            agent_hint: None,
        },
    }
}

fn pane(
    target: &str,
    session: &str,
    window: &str,
    pane_index: &str,
    command: &str,
    cwd: &str,
) -> Pane {
    Pane {
        target: target.to_string(),
        host: target.split('/').next().unwrap().to_string(),
        session: session.to_string(),
        window: window.to_string(),
        pane: pane_index.to_string(),
        pane_id: "%1".to_string(),
        pid: Some(1),
        command: command.to_string(),
        cwd: cwd.to_string(),
        session_attached: false,
        window_name: None,
        pane_title: None,
        host_short: None,
    }
}

#[test]
fn git_cache_ttl_skips_fresh_cwds_in_command() {
    let git_cache: GitCache = Arc::new(Mutex::new(HashMap::new()));
    git_cache::insert(
        &git_cache,
        ("pi".to_string(), "/home/cam/work".to_string()),
        None,
    );

    let skip = git_cache::fresh_cwds_for_host(&git_cache, "pi", Duration::from_secs(30), true);

    assert_eq!(skip, vec!["/home/cam/work".to_string()]);
    let cmd = crate::tmux::inventory_with_captures_command(2, true, &skip, None);
    assert!(cmd.contains("'/home/cam/work'"));
    assert!(cmd.contains("continue"));
}
