use crate::error::CliError;
use scope_api_contract::{ErrorCode, ErrorResponse};
use std::io::{self, IsTerminal, Write};

pub(super) fn require_confirmation(
    prompt: &str,
    yes: bool,
    interactive: bool,
) -> anyhow::Result<()> {
    if yes {
        return Ok(());
    }
    if !interactive || !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(CliError::new(ErrorResponse::new(
            ErrorCode::BadRequest,
            format!("{prompt}; rerun with --yes to confirm"),
        ))
        .into());
    }
    eprint!("{prompt}\nContinue? [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    confirmation_answer(&answer)
}

fn confirmation_answer(answer: &str) -> anyhow::Result<()> {
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err(CliError::new(ErrorResponse::new(ErrorCode::BadRequest, "cancelled")).into())
    }
}

#[cfg(test)]
mod tests {
    use super::{confirmation_answer, require_confirmation};

    #[test]
    fn yes_skips_interactive_confirmation() {
        require_confirmation("consequential action", true, true).unwrap();
    }

    #[test]
    fn noninteractive_confirmation_requires_yes() {
        let error = require_confirmation("consequential action", false, false).unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 2);
        assert_eq!(
            error.to_string(),
            "consequential action; rerun with --yes to confirm"
        );
    }

    #[test]
    fn declined_confirmation_is_a_usage_outcome() {
        let error = confirmation_answer("no").unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 2);
        assert_eq!(error.to_string(), "cancelled");
    }
}
