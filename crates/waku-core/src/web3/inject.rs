//! Prepend ProofForge / EVM skills to a prompt when a toolchain is present.

const SKILL_PROMPT_MARKER: &str = "<!-- waku:proofforge-program-v1 -->";
const EVM_SKILL_MARKER: &str = "<!-- waku:proofship-evm -->";

const PROGRAM_V1_SKILL: &str = include_str!("../../../../resources/skills/proofforge-program-v1/SKILL.md");
const EVM_SKILL: &str = include_str!("../../../../resources/skills/proofship-evm/SKILL.md");

pub fn enrich_prompt(prompt: String) -> String {
    if env_truthy("WAKU_DISABLE_PF_SKILL") || env_truthy("PROOFSHIP_DISABLE_PF_SKILL") {
        return prompt;
    }
    if super::mcp::detect_attachment().pf_mcp.is_none() && !toolchain_present() {
        return prompt;
    }
    let mut out = prompt;
    if !out.contains(SKILL_PROMPT_MARKER) {
        let skill = strip_yaml_frontmatter(PROGRAM_V1_SKILL);
        out = format!("{SKILL_PROMPT_MARKER}\n{skill}\n\n{out}");
    }
    if !out.contains(EVM_SKILL_MARKER) {
        let skill = strip_yaml_frontmatter(EVM_SKILL);
        out = format!("{EVM_SKILL_MARKER}\n{skill}\n\n{out}");
    }
    out
}

fn toolchain_present() -> bool {
    super::mcp::detect_attachment().pf_mcp.is_some()
        || super::pf_cli().is_some()
        || env_path("PF_CLI").is_some()
        || env_path("PROOF_FORGE_CLI").is_some()
        || find_on_path("proof-forge-next").is_some()
}

fn strip_yaml_frontmatter(body: &str) -> String {
    let Some(rest) = body.strip_prefix("---") else {
        return body.trim().to_string();
    };
    let Some(end) = rest.find("\n---") else {
        return body.trim().to_string();
    };
    rest[end + 4..].trim().to_string()
}

fn env_path(key: &str) -> Option<std::path::PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_file())
}

fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

fn env_truthy(key: &str) -> bool {
    matches!(
        std::env::var(key).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_prevents_double_inject_when_forced() {
        let already = format!("{SKILL_PROMPT_MARKER}\nhello");
        let out = enrich_prompt(already.clone());
        assert_eq!(out.matches(SKILL_PROMPT_MARKER).count(), 1);
    }

    #[test]
    fn frontmatter_is_stripped() {
        let body = "---\nname: x\n---\n\n# Title\n";
        assert_eq!(strip_yaml_frontmatter(body), "# Title");
    }
}
