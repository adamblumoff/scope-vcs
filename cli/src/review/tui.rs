use super::state::{ReviewInput, ReviewMode, ReviewRow, ReviewState, ReviewStateAction};
use anyhow::Context;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{prelude::*, widgets::Paragraph};
use scope_core::domain::repo_config::RepoConfig;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiOutcome {
    Exit,
    Cancel,
    ContinuePush,
}

pub fn run_review_tui(
    mut state: ReviewState,
    mut save_config: impl FnMut(&RepoConfig) -> anyhow::Result<()>,
) -> anyhow::Result<TuiOutcome> {
    enable_raw_mode().context("enable terminal raw mode")?;
    let _guard = TerminalRestoreGuard;
    execute!(io::stdout(), EnterAlternateScreen).context("enter alternate terminal screen")?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("open terminal UI")?;

    loop {
        terminal
            .draw(|frame| render(frame, &mut state))
            .context("draw Scope review UI")?;

        let Event::Key(key) = event::read().context("read terminal input")? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let Some(input) = key_to_input(&state, key) else {
            continue;
        };
        match state.handle_input(input) {
            ReviewStateAction::None => {}
            ReviewStateAction::Save => {
                if state.is_dirty() {
                    save_config(&state.config)?;
                    state.mark_saved();
                }
            }
            ReviewStateAction::ContinuePush => {
                if state.is_dirty() {
                    save_config(&state.config)?;
                }
                return Ok(TuiOutcome::ContinuePush);
            }
            ReviewStateAction::Exit => return Ok(TuiOutcome::Exit),
            ReviewStateAction::Cancel => return Ok(TuiOutcome::Cancel),
        }
    }
}

fn render(frame: &mut Frame<'_>, state: &mut ReviewState) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);

    let mode = match state.mode() {
        ReviewMode::Standalone => "scope review",
        ReviewMode::Push => "scope push review",
    };
    let dirty = if state.is_dirty() {
        "modified"
    } else {
        "clean"
    };
    let filter = if state.filter().is_empty() {
        "filter: none".to_string()
    } else {
        format!("filter: {}", terminal_safe(state.filter()))
    };
    let filter_mode = if state.editing_filter() {
        " editing"
    } else {
        ""
    };
    let rewrite_note = if state.history_rewrite_count() == 0 {
        String::new()
    } else {
        format!(
            "  history rewrites: {} read-only",
            state.history_rewrite_count()
        )
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!("{mode}  .scope/repo.json  {dirty}{rewrite_note}")),
            Line::from(format!(
                "Path                                      Visibility  Rule  {filter}{filter_mode}"
            )),
            Line::from(
                "--------------------------------------------------------------------------",
            ),
        ]),
        chunks[0],
    );

    let body_height = chunks[1].height as usize;
    let read_only_lines = state
        .history_rewrite_summaries()
        .into_iter()
        .chain(state.deleted_path_summaries().iter().cloned())
        .map(|line| Line::from(terminal_safe(&line)))
        .collect::<Vec<_>>();
    let rows = state.visible_rows();
    let (read_only_height, row_height) =
        review_body_heights(body_height, read_only_lines.len(), rows.len());
    state.adjust_scroll(row_height);
    let mut lines = read_only_lines
        .into_iter()
        .take(read_only_height)
        .collect::<Vec<_>>();
    lines.extend(
        rows.iter()
            .enumerate()
            .skip(state.scroll())
            .take(row_height)
            .map(|(index, row)| row_line(row, index == state.cursor())),
    );
    frame.render_widget(Paragraph::new(lines), chunks[1]);

    let push_hint = if state.mode() == ReviewMode::Push {
        "  P continue push"
    } else {
        ""
    };
    let quit_label = if state.mode() == ReviewMode::Push {
        "Q cancel"
    } else {
        "Q quit"
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!(
                "Space toggle  Right expand  Left collapse  S save{push_hint}  {quit_label}  / filter  ? help"
            )),
            Line::from(terminal_safe(state.message())),
        ]),
        chunks[2],
    );
}

fn review_body_heights(
    body_height: usize,
    read_only_line_count: usize,
    row_count: usize,
) -> (usize, usize) {
    let reserved_row_height = usize::from(body_height > 0 && row_count > 0);
    let read_only_height =
        read_only_line_count.min(body_height.saturating_sub(reserved_row_height));
    (
        read_only_height,
        body_height.saturating_sub(read_only_height),
    )
}

fn row_line(row: &ReviewRow, selected: bool) -> Line<'static> {
    let symbol = match row.kind {
        super::tree::ReviewNodeKind::Root => " . ",
        super::tree::ReviewNodeKind::File => "   ",
        super::tree::ReviewNodeKind::Directory if row.expanded => "[v]",
        super::tree::ReviewNodeKind::Directory => "[>]",
    };
    let indent = "  ".repeat(row.depth);
    let change = row
        .change_status
        .as_ref()
        .map(|status| format!("  {status}"))
        .unwrap_or_default();
    let reserved = if row.reserved { " reserved" } else { "" };
    let text = format!(
        "{indent}{symbol} {:<34} {:<10} {}{}{}",
        terminal_safe(&row.name),
        super::policy::visibility_label(row.visibility),
        terminal_safe(&row.rule),
        reserved,
        terminal_safe(&change)
    );
    if selected {
        Line::from(text).style(Style::new().add_modifier(Modifier::REVERSED))
    } else {
        Line::from(text)
    }
}

fn terminal_safe(text: &str) -> String {
    text.chars().flat_map(char::escape_default).collect()
}

fn key_to_input(state: &ReviewState, key: KeyEvent) -> Option<ReviewInput> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(ReviewInput::Escape);
    }
    if state.editing_filter() {
        return match key.code {
            KeyCode::Esc | KeyCode::Enter => Some(ReviewInput::Escape),
            KeyCode::Backspace => Some(ReviewInput::Backspace),
            KeyCode::Char(value) => Some(ReviewInput::Char(value)),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Up => Some(ReviewInput::Up),
        KeyCode::Down => Some(ReviewInput::Down),
        KeyCode::Left => Some(ReviewInput::Left),
        KeyCode::Right => Some(ReviewInput::Right),
        KeyCode::Char(' ') => Some(ReviewInput::Toggle),
        KeyCode::Char('s') | KeyCode::Char('S') => Some(ReviewInput::Save),
        KeyCode::Char('p') | KeyCode::Char('P') => Some(ReviewInput::ContinuePush),
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(ReviewInput::Quit),
        KeyCode::Char('/') => Some(ReviewInput::Filter),
        KeyCode::Char('?') => Some(ReviewInput::Help),
        KeyCode::Esc => Some(ReviewInput::Escape),
        _ => None,
    }
}

struct TerminalRestoreGuard;

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
