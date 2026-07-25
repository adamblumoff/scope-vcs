use anyhow::bail;
use std::io::{self, IsTerminal, Write};

pub(super) fn require_confirmation(prompt: &str, yes: bool) -> anyhow::Result<()> {
    if yes {
        return Ok(());
    }
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        bail!("{prompt}; rerun with --yes to confirm");
    }
    eprint!("{prompt}\nContinue? [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        bail!("cancelled")
    }
}

#[cfg(test)]
mod tests {
    use super::require_confirmation;

    #[test]
    fn yes_skips_interactive_confirmation() {
        require_confirmation("consequential action", true).unwrap();
    }

    #[test]
    fn noninteractive_confirmation_requires_yes() {
        let error = require_confirmation("consequential action", false)
            .unwrap_err()
            .to_string();
        assert_eq!(error, "consequential action; rerun with --yes to confirm");
    }
}
