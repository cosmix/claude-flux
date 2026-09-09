//! Skill recommendation section of a stage signal: per-skill invocations plus one combined loader call.

use crate::skills::{is_core_skill, skill_invocation, SkillMatch};

/// Format task progression information for inclusion in signals
pub fn format_skill_recommendations(skills: &[SkillMatch]) -> String {
    let mut content = String::new();

    content.push_str("## Recommended Skills\n\n");

    // Partition skills into two classes with different framing:
    // - `detected`: language skills inferred from the files this stage edits.
    //   These are a DIRECTIVE — load them before writing code.
    // - `advisory`: skills matched from the task description. Invoke if relevant.
    let (detected, advisory): (Vec<&SkillMatch>, Vec<&SkillMatch>) = skills.iter().partition(|s| {
        s.matched_triggers
            .iter()
            .any(|t| t == "project-language" || t.starts_with("project-type:"))
    });

    if !detected.is_empty() {
        content.push_str(&format_detected_skills(&detected));
    }

    if !advisory.is_empty() {
        content.push_str(&format_advisory_skills(&advisory));
    }

    // One combined loader call for every catalogued skill recommended above,
    // detected first then advisory, so an agent can load them all in a
    // single Skill tool invocation instead of one per skill.
    let ordered: Vec<&SkillMatch> = detected
        .iter()
        .copied()
        .chain(advisory.iter().copied())
        .collect();
    if let Some(line) = combined_loader_line(&ordered) {
        content.push_str(&line);
    }

    content
}

/// Render the "load now" directive block for skills inferred from the file
/// types this stage edits.
fn format_detected_skills(detected: &[&SkillMatch]) -> String {
    let mut content = String::new();
    content.push_str(
        "**Load these now — before editing any files.** Based on the file types this \
         stage will edit, invoke the Skill tool for each so your code follows the \
         project's language and framework conventions:\n\n",
    );
    // Claude Code indexes only the core skills, so a catalogued one has no
    // `Skill(skill="loom-rust")` of its own. `skill_invocation` renders the
    // loom-skills loader call for those, and the plain call for the rest.
    for skill in detected {
        content.push_str(&format!("- `{}`\n", skill_invocation(&skill.name)));
    }
    content.push('\n');
    content
}

/// Render the advisory table + matched-triggers block for skills matched
/// from the task description.
fn format_advisory_skills(advisory: &[&SkillMatch]) -> String {
    let mut content = String::new();
    content.push_str("These skills may also help with your task — invoke any that apply:\n\n");
    content.push_str("| Skill | Description | Invoke |\n");
    content.push_str("|-------|-------------|--------|\n");

    for skill in advisory {
        // Truncate description if too long for table (UTF-8 safe)
        let desc = if skill.description.chars().count() > 60 {
            format!(
                "{}...",
                skill.description.chars().take(57).collect::<String>()
            )
        } else {
            skill.description.clone()
        };
        // Escape pipe characters in description and name
        let desc = desc.replace('|', "\\|");
        let invoke = skill_invocation(&skill.name);
        let name = skill.name.replace('|', "\\|");
        content.push_str(&format!("| {} | {} | `{}` |\n", name, desc, invoke));
    }
    content.push('\n');

    // Show which triggers matched for transparency
    content.push_str("**Matched triggers:**\n");
    for skill in advisory {
        if !skill.matched_triggers.is_empty() {
            let triggers = skill.matched_triggers.join(", ");
            content.push_str(&format!("- `{}`: {}\n", skill.name, triggers));
        }
    }
    content.push('\n');

    content
}

/// One combined `loom-skills` loader call naming every catalogued (non-core)
/// skill in `skills`, in the given order. `None` when fewer than two qualify
/// — a single catalogued skill already has its own invocation line above,
/// and a core skill has no catalog entry for the loader to read.
fn combined_loader_line(skills: &[&SkillMatch]) -> Option<String> {
    let names: Vec<&str> = skills
        .iter()
        .map(|s| s.name.as_str())
        .filter(|name| !is_core_skill(name))
        .collect();

    if names.len() < 2 {
        return None;
    }

    Some(format!(
        "**Load all catalogued ones at once:** `Skill(skill=\"loom-skills\", args=\"{}\")`\n\n",
        names.join(" ")
    ))
}

#[cfg(test)]
mod skill_recommendation_tests {
    use super::format_skill_recommendations;
    use crate::skills::SkillMatch;

    fn detected(name: &str) -> SkillMatch {
        SkillMatch::new(
            name.to_string(),
            "Language expertise".to_string(),
            10.0,
            vec!["project-language".to_string()],
        )
    }

    fn advisory(name: &str, trigger: &str) -> SkillMatch {
        SkillMatch::new(
            name.to_string(),
            "Some advisory skill".to_string(),
            2.0,
            vec![trigger.to_string()],
        )
    }

    #[test]
    fn detected_skills_render_as_skill_tool_directive() {
        let out = format_skill_recommendations(&[detected("loom-rust")]);
        // Directive framing + an explicit Skill tool invocation the agent can run.
        assert!(out.contains("Load these now"), "missing directive: {out}");
        assert!(
            out.contains("Skill(skill=\"loom-skills\", args=\"loom-rust\")"),
            "missing Skill tool call: {out}"
        );
    }

    #[test]
    fn advisory_skills_render_as_table_not_directive() {
        let out = format_skill_recommendations(&[advisory("loom-auth", "jwt")]);
        assert!(
            !out.contains("Load these now"),
            "should not be directive: {out}"
        );
        assert!(
            out.contains("may also help"),
            "missing advisory framing: {out}"
        );
        assert!(
            out.contains("Skill(skill=\"loom-skills\", args=\"loom-auth\")"),
            "missing invoke column: {out}"
        );
        assert!(out.contains("jwt"), "missing matched trigger: {out}");
    }

    #[test]
    fn detected_and_advisory_are_partitioned() {
        let out =
            format_skill_recommendations(&[detected("loom-rust"), advisory("loom-auth", "jwt")]);
        // Detected directive comes before the advisory table.
        let load_pos = out.find("Load these now").expect("directive present");
        let advisory_pos = out.find("may also help").expect("advisory present");
        assert!(
            load_pos < advisory_pos,
            "directive should precede advisory: {out}"
        );
    }

    #[test]
    fn two_catalogued_skills_get_a_combined_loader_line() {
        let out =
            format_skill_recommendations(&[detected("loom-rust"), advisory("loom-auth", "jwt")]);
        assert!(
            out.contains(
                "**Load all catalogued ones at once:** \
                 `Skill(skill=\"loom-skills\", args=\"loom-rust loom-auth\")`"
            ),
            "missing combined loader line in detected-then-advisory order: {out}"
        );
    }

    #[test]
    fn single_catalogued_skill_has_no_combined_loader_line() {
        let out = format_skill_recommendations(&[detected("loom-rust")]);
        assert!(
            !out.contains("Load all catalogued ones at once"),
            "combined line should not appear for a single skill: {out}"
        );
    }

    #[test]
    fn core_skill_is_excluded_from_combined_loader_line() {
        // loom-plan-writer is a core skill (skills/core-skills.txt) — it has
        // no catalog entry and should never appear in the combined args.
        let out = format_skill_recommendations(&[
            detected("loom-rust"),
            advisory("loom-plan-writer", "plan"),
        ]);
        assert!(
            !out.contains("Load all catalogued ones at once"),
            "combined line should not appear with only one catalogued skill: {out}"
        );
        let out2 = format_skill_recommendations(&[
            detected("loom-rust"),
            detected("loom-auth"),
            advisory("loom-plan-writer", "plan"),
        ]);
        assert!(
            out2.contains("args=\"loom-rust loom-auth\""),
            "combined args should hold only catalogued skills: {out2}"
        );
    }
}
