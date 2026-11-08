// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Resolver for mapping cgroup IDs (inodes) to relative cgroup paths.
pub struct CgroupResolver {
    cgroup_root: PathBuf,
    cache: HashMap<u64, (String, Instant)>,
    cache_ttl_secs: u64,
}

impl CgroupResolver {
    pub fn new() -> Self {
        Self::with_root(PathBuf::from("/sys/fs/cgroup"))
    }

    pub fn with_root(cgroup_root: PathBuf) -> Self {
        Self {
            cgroup_root,
            cache: HashMap::new(),
            cache_ttl_secs: 60,
        }
    }

    /// Resolve a cgroup inode ID to a cgroup directory path.
    pub fn resolve_cgroup_id(&mut self, cgroup_id: u64) -> Option<String> {
        if cgroup_id == 0 {
            return None;
        }

        // Check cache
        if let Some((path, cached_at)) = self.cache.get(&cgroup_id) {
            if cached_at.elapsed().as_secs() < self.cache_ttl_secs {
                return Some(path.clone());
            }
        }

        // Walk cgroup directory hierarchy to find matching inode
        if let Some(path) = self.scan_for_inode(&self.cgroup_root.clone(), cgroup_id) {
            let rel = path
                .strip_prefix(&self.cgroup_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let final_path = if rel.is_empty() {
                "/".to_string()
            } else {
                format!("/{}", rel)
            };
            self.cache
                .insert(cgroup_id, (final_path.clone(), Instant::now()));
            return Some(final_path);
        }

        None
    }

    fn scan_for_inode(&self, dir: &Path, target_inode: u64) -> Option<PathBuf> {
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(meta) = entry.metadata() {
                if meta.ino() == target_inode {
                    return Some(path);
                }
                if meta.is_dir() {
                    if let Some(found) = self.scan_for_inode(&path, target_inode) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }
}

impl Default for CgroupResolver {
    fn default() -> Self {
        Self::new()
    }
}
