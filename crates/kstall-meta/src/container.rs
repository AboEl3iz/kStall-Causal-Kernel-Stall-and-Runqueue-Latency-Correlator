// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContainerMeta {
    pub container_id: Option<String>,
    pub pod_name: Option<String>,
    pub pod_namespace: Option<String>,
}

/// Parse container and Kubernetes metadata from a cgroup path.
pub fn parse_cgroup_metadata(cgroup_path: &str) -> ContainerMeta {
    let mut meta = ContainerMeta::default();

    let segments: Vec<&str> = cgroup_path.split('/').filter(|s| !s.is_empty()).collect();

    for (i, segment) in segments.iter().enumerate() {
        if segment.starts_with("pod") || segment.starts_with("kubepods-") {
            if let Some(pod_uid) = extract_pod_uid(segment) {
                meta.pod_name = Some(pod_uid);
            }
        }

        // Docker / Containerd / Podman with systemd driver
        if segment.starts_with("cri-containerd-")
            || segment.starts_with("crio-")
            || segment.starts_with("docker-")
            || segment.starts_with("libpod-")
            || segment.starts_with("podman-")
        {
            let id = segment
                .trim_start_matches("cri-containerd-")
                .trim_start_matches("crio-")
                .trim_start_matches("docker-")
                .trim_start_matches("libpod-")
                .trim_start_matches("podman-")
                .trim_end_matches(".scope");
            if !id.is_empty() {
                meta.container_id = Some(id.to_string());
            }
        } else if is_container_id(segment) {
            // Standalone Docker cgroupfs driver: /docker/<64_hex_id>
            meta.container_id = Some(segment.to_string());
        } else if *segment == "docker" && i + 1 < segments.len() {
            let next = segments[i + 1];
            if !next.is_empty() {
                meta.container_id = Some(next.trim_end_matches(".scope").to_string());
            }
        }
    }

    meta
}

fn is_container_id(s: &str) -> bool {
    let cleaned = s.trim_end_matches(".scope");
    cleaned.len() == 64 && cleaned.chars().all(|c| c.is_ascii_hexdigit())
}

fn extract_pod_uid(segment: &str) -> Option<String> {
    let pod_idx = if let Some(idx) = segment.find("-pod") {
        idx + 4
    } else if segment.starts_with("pod") {
        3
    } else {
        return None;
    };

    let rest = &segment[pod_idx..];
    let cleaned = rest.trim_end_matches(".slice");
    let uid = cleaned.replace('_', "-");
    if !uid.is_empty() {
        Some(uid)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_k8s_cgroup() {
        let path = "/sys/fs/cgroup/kubepods.slice/kubepods-burstable.slice/kubepods-burstable-pod7d174661_37d5_4c19_9d1f_a18b76c8c977.slice/cri-containerd-6d55787926bdf.scope";
        let meta = parse_cgroup_metadata(path);
        assert!(meta.container_id.is_some());
        assert_eq!(meta.container_id.unwrap(), "6d55787926bdf");
        assert!(meta.pod_name.is_some());
        assert_eq!(
            meta.pod_name.unwrap(),
            "7d174661-37d5-4c19-9d1f-a18b76c8c977"
        );
    }

    #[test]
    fn test_parse_docker_standalone_cgroups() {
        // Systemd driver: /sys/fs/cgroup/system.slice/docker-e9a8f4c1b2.scope
        let path_systemd = "/sys/fs/cgroup/system.slice/docker-e9a8f4c1b2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9.scope";
        let meta_sd = parse_cgroup_metadata(path_systemd);
        assert_eq!(
            meta_sd.container_id.as_deref(),
            Some("e9a8f4c1b2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9")
        );
        assert!(meta_sd.pod_name.is_none());

        // Cgroupfs driver: /sys/fs/cgroup/docker/e9a8f4c1b2...
        let path_cgroupfs = "/sys/fs/cgroup/docker/e9a8f4c1b2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9";
        let meta_cg = parse_cgroup_metadata(path_cgroupfs);
        assert_eq!(
            meta_cg.container_id.as_deref(),
            Some("e9a8f4c1b2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9")
        );
    }
}
