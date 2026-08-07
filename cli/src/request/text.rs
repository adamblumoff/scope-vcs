use anyhow::{Context, bail};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(super) fn short_oid(oid: &str) -> String {
    oid.chars().take(12).collect()
}

pub(super) fn terminal_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

pub(super) fn discussion_body(
    body: Option<String>,
    body_file: Option<PathBuf>,
) -> anyhow::Result<String> {
    let mut stdin = io::stdin().lock();
    discussion_body_with_stdin(body, body_file, &mut stdin)
}

pub(super) fn discussion_body_with_stdin(
    body: Option<String>,
    body_file: Option<PathBuf>,
    stdin: &mut dyn Read,
) -> anyhow::Result<String> {
    match (body, body_file) {
        (Some(body), None) => Ok(body),
        (None, Some(path)) if path == Path::new("-") => {
            let mut body = String::new();
            stdin
                .read_to_string(&mut body)
                .context("read discussion body from stdin")?;
            Ok(body)
        }
        (None, Some(path)) => fs::read_to_string(&path)
            .with_context(|| format!("read discussion body from {}", path.display())),
        _ => bail!("exactly one of --body or --body-file is required"),
    }
}
