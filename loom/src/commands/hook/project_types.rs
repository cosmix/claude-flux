//! Project facts for the skill-trigger hook; no model or network calls.

use std::io::{self, Read};
use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;
use serde_json::json;

use crate::skills::project::ProjectProfile;

#[derive(Deserialize)]
struct Input {
    cwd: PathBuf,
    #[serde(default)]
    prompt: String,
}

pub fn execute() -> Result<()> {
    let mut input = String::new();
    io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    if input.len() > 1024 * 1024 {
        return Ok(());
    }
    let input: Input = serde_json::from_str(&input)?;
    let cwd = input.cwd.canonicalize()?;
    let profile = ProjectProfile::discover(&cwd);
    let types = profile.for_prompt(&cwd, &input.prompt);
    println!(
        "{}",
        json!({"root": profile.root, "types": types, "truncated": profile.truncated})
    );
    Ok(())
}
