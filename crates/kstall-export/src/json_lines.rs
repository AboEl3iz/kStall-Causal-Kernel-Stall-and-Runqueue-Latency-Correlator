// SPDX-License-Identifier: (LGPL-2.1 OR BSD-2-Clause)
// Copyright (c) 2026 kStall Developers

use anyhow::Result;
use kstall_core::model::Episode;
use std::io::Write;

/// Write a single episode as a JSON line.
pub fn write_episode_json_line<W: Write>(mut writer: W, episode: &Episode) -> Result<()> {
    let json = serde_json::to_string(episode)?;
    writeln!(writer, "{}", json)?;
    writer.flush()?;
    Ok(())
}
