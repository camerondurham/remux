use super::GitCache;
use crate::git::RepoSnapshot;
use std::time::{Duration, Instant};

pub(super) fn fresh_cwds_for_host(
    git_cache: &GitCache,
    host_id: &str,
    ttl: Duration,
    collect_git: bool,
) -> Vec<String> {
    if !collect_git {
        return Vec::new();
    }

    let cache = git_cache.lock().expect("git cache poisoned");
    cache
        .iter()
        .filter(|((cached_host, _), (_, fetched))| {
            cached_host.as_str() == host_id && fetched.elapsed() < ttl
        })
        .map(|((_, cwd), _)| cwd.clone())
        .collect()
}

pub(super) fn get_fresh(
    git_cache: &GitCache,
    key: &(String, String),
    ttl: Duration,
) -> Option<Option<RepoSnapshot>> {
    let cache = git_cache.lock().expect("git cache poisoned");
    cache
        .get(key)
        .and_then(|(snapshot, fetched)| (fetched.elapsed() < ttl).then(|| snapshot.clone()))
}

pub(super) fn insert(git_cache: &GitCache, key: (String, String), snapshot: Option<RepoSnapshot>) {
    let mut cache = git_cache.lock().expect("git cache poisoned");
    cache.insert(key, (snapshot, Instant::now()));
}
