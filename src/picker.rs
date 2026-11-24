//! Interactive fuzzy picker drawn on the controlling terminal, so it works while stdout
//! is redirected (`focal files > out.md`).
//!
//! [`State`] holds the query, matches, cursor, and marks without any I/O, so selection,
//! filtering, and scrolling are unit-tested directly. [`render`] draws a state into any
//! ratatui backend, and [`pick`] runs the event loop on `/dev/tty`.

use crate::preview::Previewer;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::{cursor, execute};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::{Backend, CrosstermBackend};
#[cfg(test)]
use ratatui::layout::Rect;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use std::collections::{BTreeSet, HashMap};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::sync::Arc;
use std::time::Duration;

/// One candidate. The caller keeps the selected value; the picker returns item indices.
pub struct Item {
    /// Text shown in the list.
    pub label: String,
    /// Text matched against the query.
    pub search: String,
    /// Approximate payload size in bytes, used for the running token estimate.
    pub weight: u64,
}

pub struct Options {
    pub prompt: String,
    pub header: Vec<String>,
    pub multi: bool,
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Indices of the chosen items in input order (possibly empty when nothing matched).
    Selected(Vec<usize>),
    Cancelled,
}

#[derive(Debug, PartialEq)]
pub enum Step {
    Continue,
    Done(Outcome),
}

pub struct State {
    items: Vec<Item>,
    multi: bool,
    query: String,
    matches: Vec<u32>,
    cursor: usize,
    offset: usize,
    marked: BTreeSet<u32>,
    page: usize,
    preview_scroll: u16,
    matcher: Matcher,
}

impl State {
    pub fn new(items: Vec<Item>, multi: bool) -> Self {
        let matches = (0..items.len() as u32).collect();
        Self {
            items,
            multi,
            query: String::new(),
            matches,
            cursor: 0,
            offset: 0,
            marked: BTreeSet::new(),
            page: 10,
            preview_scroll: 0,
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
        }
    }

    /// The item under the cursor.
    pub fn current(&self) -> Option<usize> {
        self.matches.get(self.cursor).map(|&id| id as usize)
    }

    pub fn marked(&self) -> Vec<usize> {
        self.marked.iter().map(|&id| id as usize).collect()
    }

    #[cfg(test)]
    pub fn visible_matches(&self) -> Vec<usize> {
        self.matches.iter().map(|&id| id as usize).collect()
    }

    /// Re-scores every item against the query, keeping the cursor on the same item when it
    /// still matches. Ties keep input order.
    fn refilter(&mut self) {
        let keep = self.current();
        if self.query.is_empty() {
            self.matches = (0..self.items.len() as u32).collect();
        } else {
            let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
            let mut buf = Vec::new();
            let mut scored: Vec<(u32, u32)> = self
                .items
                .iter()
                .enumerate()
                .filter_map(|(id, item)| {
                    pattern
                        .score(Utf32Str::new(&item.search, &mut buf), &mut self.matcher)
                        .map(|score| (score, id as u32))
                })
                .collect();
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            self.matches = scored.into_iter().map(|(_, id)| id).collect();
        }
        self.cursor = keep
            .and_then(|id| self.matches.iter().position(|&m| m as usize == id))
            .unwrap_or(0);
        self.offset = 0;
        self.preview_scroll = 0;
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.matches.is_empty() {
            return;
        }
        let last = self.matches.len() as isize - 1;
        let next = (self.cursor as isize + delta).clamp(0, last) as usize;
        if next != self.cursor {
            self.cursor = next;
            self.preview_scroll = 0;
        }
    }

    fn toggle_current(&mut self) {
        if let Some(id) = self.matches.get(self.cursor).copied()
            && !self.marked.remove(&id)
        {
            self.marked.insert(id);
        }
    }

    /// Applies one key press, fzf style.
    pub fn handle(&mut self, key: KeyEvent) -> Step {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Esc => return Step::Done(Outcome::Cancelled),
            KeyCode::Char('c' | 'g' | 'q') if ctrl => return Step::Done(Outcome::Cancelled),
            KeyCode::Enter => {
                let chosen = if self.multi && !self.marked.is_empty() {
                    self.marked()
                } else {
                    self.current().into_iter().collect()
                };
                return Step::Done(Outcome::Selected(chosen));
            }
            KeyCode::Tab if self.multi => {
                self.toggle_current();
                self.move_cursor(1);
            }
            KeyCode::BackTab if self.multi => {
                self.toggle_current();
                self.move_cursor(-1);
            }
            KeyCode::Char('a') if ctrl && self.multi => self.marked.extend(self.matches.iter()),
            KeyCode::Char('d') if ctrl && self.multi => {
                for id in &self.matches {
                    self.marked.remove(id);
                }
            }
            KeyCode::Up if shift => self.preview_scroll = self.preview_scroll.saturating_sub(1),
            KeyCode::Down if shift => self.preview_scroll = self.preview_scroll.saturating_add(1),
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Char('k' | 'p') if ctrl => self.move_cursor(-1),
            KeyCode::Char('j' | 'n') if ctrl => self.move_cursor(1),
            KeyCode::PageUp => self.move_cursor(-(self.page as isize)),
            KeyCode::PageDown => self.move_cursor(self.page as isize),
            KeyCode::Home => self.move_cursor(isize::MIN / 2),
            KeyCode::End => self.move_cursor(isize::MAX / 2),
            KeyCode::Backspace => {
                if self.query.pop().is_some() {
                    self.refilter();
                }
            }
            KeyCode::Char('u') if ctrl => {
                self.query.clear();
                self.refilter();
            }
            KeyCode::Char('w') if ctrl => {
                let trimmed = self.query.trim_end();
                let cut = trimmed.rfind(char::is_whitespace).map_or(0, |i| i + 1);
                self.query.truncate(cut);
                self.refilter();
            }
            KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.query.push(c);
                self.refilter();
            }
            _ => {}
        }
        Step::Continue
    }

    /// Keeps the cursor row inside a list area of `height` rows.
    fn scroll_into_view(&mut self, height: usize) {
        self.page = height.max(1);
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if height > 0 && self.cursor >= self.offset + height {
            self.offset = self.cursor + 1 - height;
        }
    }

    fn highlighted(&mut self, id: usize) -> Vec<usize> {
        if self.query.is_empty() {
            return Vec::new();
        }
        let pattern = Pattern::parse(&self.query, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        let mut indices = Vec::new();
        pattern.indices(
            Utf32Str::new(&self.items[id].search, &mut buf),
            &mut self.matcher,
            &mut indices,
        );
        indices.sort_unstable();
        indices.dedup();
        indices.into_iter().map(|i| i as usize).collect()
    }
}

/// Draws the picker: prompt, counts, header lines, then the list beside the preview.
pub fn render(frame: &mut Frame, state: &mut State, options: &Options, preview: Option<&str>) {
    let area = frame.area();
    let header_rows = options.header.len() as u16;
    let [prompt_row, info_row, header_area, body] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(header_rows),
        Constraint::Min(1),
    ])
    .areas(area);
    let [list_area, preview_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(body);

    let accent = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
    let dim = Style::new().fg(Color::DarkGray);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(options.prompt.clone(), accent),
            Span::raw(state.query.clone()),
        ])),
        prompt_row,
    );
    let cursor_x =
        prompt_row.x + (options.prompt.chars().count() + state.query.chars().count()) as u16;
    frame.set_cursor_position((
        cursor_x.min(prompt_row.right().saturating_sub(1)),
        prompt_row.y,
    ));

    let mut info = format!("  {}/{}", state.matches.len(), state.items.len());
    if state.multi {
        let bytes: u64 = state
            .marked
            .iter()
            .map(|&id| state.items[id as usize].weight)
            .sum();
        info.push_str(&format!(
            " ({} selected, ~{} tokens)",
            state.marked.len(),
            bytes / 4
        ));
    }
    frame.render_widget(Paragraph::new(Span::styled(info, dim)), info_row);
    let header: Vec<Line> = options
        .header
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), dim)))
        .collect();
    frame.render_widget(Paragraph::new(header), header_area);

    let height = list_area.height as usize;
    state.scroll_into_view(height);
    let rows: Vec<usize> = state
        .matches
        .iter()
        .skip(state.offset)
        .take(height)
        .map(|&id| id as usize)
        .collect();
    let mut lines = Vec::with_capacity(rows.len());
    for (row, id) in rows.into_iter().enumerate() {
        let on_cursor = state.offset + row == state.cursor;
        let marked = state.marked.contains(&(id as u32));
        let base = if on_cursor {
            Style::new()
                .add_modifier(Modifier::BOLD)
                .bg(Color::DarkGray)
        } else {
            Style::new()
        };
        let hits = state.highlighted(id);
        let mut spans = vec![
            Span::styled(if on_cursor { ">" } else { " " }, base.fg(Color::Red)),
            Span::styled(if marked { "+" } else { " " }, base.fg(Color::Magenta)),
            Span::styled(" ", base),
        ];
        for (index, ch) in state.items[id].label.chars().enumerate() {
            let style = if hits.binary_search(&index).is_ok() {
                base.fg(Color::Green)
            } else {
                base
            };
            spans.push(Span::styled(ch.to_string(), style));
        }
        lines.push(Line::from(spans));
    }
    if state.matches.is_empty() {
        lines.push(Line::from(Span::styled("  no matches", dim)));
    }
    frame.render_widget(Paragraph::new(lines), list_area);

    let title = state
        .current()
        .map(|id| format!(" {} ", state.items[id].label))
        .unwrap_or_default();
    let text = match (state.current(), preview) {
        (None, _) => String::new(),
        (Some(_), Some(text)) => text.to_owned(),
        (Some(_), None) => "Loading preview...".to_owned(),
    };
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::bordered().title(title).border_style(dim))
            .scroll((state.preview_scroll, 0)),
        preview_area,
    );
}

/// Restores the terminal however the picker exits, including during a panic.
struct Restore {
    tty: File,
}

impl Drop for Restore {
    fn drop(&mut self) {
        let _ = execute!(self.tty, LeaveAlternateScreen, cursor::Show);
        let _ = terminal::disable_raw_mode();
        let _ = self.tty.flush();
    }
}

/// Runs the picker on the controlling terminal. `preview` renders an item's preview text;
/// it runs on a worker thread so a slow document never blocks typing or navigation.
pub fn pick(
    items: Vec<Item>,
    options: &Options,
    preview: Option<Arc<dyn Fn(usize) -> String + Send + Sync>>,
) -> io::Result<Outcome> {
    let tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .map_err(|error| {
            io::Error::other(format!(
                "interactive selection needs a terminal ({error}); pass paths or a flag such as --all instead"
            ))
        })?;
    if items.is_empty() {
        return Ok(Outcome::Selected(Vec::new()));
    }
    let mut state = State::new(items, options.multi);
    let previewer = preview.map(Previewer::new);
    let mut previews: HashMap<usize, String> = HashMap::new();

    terminal::enable_raw_mode()?;
    let mut guard = Restore { tty };
    execute!(guard.tty, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(guard.tty.try_clone()?))?;
    let mut requested = None;
    loop {
        let current = state.current();
        if let Some(previewer) = &previewer {
            while let Some((id, text)) = previewer.try_result() {
                previews.insert(id, text);
            }
            if let Some(id) = current
                && requested != Some(id)
                && !previews.contains_key(&id)
            {
                previewer.request(id);
                requested = Some(id);
            }
        }
        let shown = current.and_then(|id| previews.get(&id)).map(String::as_str);
        draw(
            &mut terminal,
            &mut state,
            options,
            shown,
            previewer.is_some(),
        )?;
        if !event::poll(Duration::from_millis(50))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if let Step::Done(outcome) = state.handle(key) {
                    drop(terminal);
                    drop(guard);
                    return Ok(outcome);
                }
            }
            Event::Resize(..) => terminal.autoresize()?,
            _ => {}
        }
    }
}

/// Lets the user choose files from `candidates`, previewing each one. Returns `None` when
/// the picker is cancelled.
pub fn pick_paths(
    candidates: Vec<std::path::PathBuf>,
    prompt: &str,
    header: Vec<String>,
) -> io::Result<Option<Vec<std::path::PathBuf>>> {
    let items = candidates
        .iter()
        .map(|path| {
            let label = path.display().to_string();
            Item {
                search: label.clone(),
                label,
                weight: std::fs::metadata(path).map_or(0, |meta| meta.len()),
            }
        })
        .collect();
    let paths = Arc::new(candidates);
    let noise = Arc::new(crate::file::Noise::load());
    let shared = Arc::clone(&paths);
    let loader: Arc<dyn Fn(usize) -> String + Send + Sync> =
        Arc::new(move |id| crate::preview::file(&shared[id], &noise));
    let options = Options {
        prompt: prompt.to_owned(),
        header,
        multi: true,
    };
    Ok(match pick(items, &options, Some(loader))? {
        Outcome::Cancelled => None,
        Outcome::Selected(ids) => Some(ids.into_iter().map(|id| paths[id].clone()).collect()),
    })
}

fn draw<B: Backend>(
    terminal: &mut Terminal<B>,
    state: &mut State,
    options: &Options,
    preview: Option<&str>,
    has_preview: bool,
) -> io::Result<()>
where
    B::Error: Into<io::Error>,
{
    terminal
        .draw(|frame| {
            let shown = if has_preview { preview } else { Some("") };
            render(frame, state, options, shown);
        })
        .map(|_| ())
        .map_err(Into::into)
}

/// The area the list occupies for a terminal of the given size, for tests.
#[cfg(test)]
pub fn list_height(area: Rect, header_rows: u16) -> u16 {
    area.height.saturating_sub(2 + header_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn items(names: &[&str]) -> Vec<Item> {
        names
            .iter()
            .map(|name| Item {
                label: (*name).into(),
                search: (*name).into(),
                weight: 40,
            })
            .collect()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn typed(state: &mut State, text: &str) {
        for c in text.chars() {
            state.handle(key(KeyCode::Char(c)));
        }
    }

    const FILES: [&str; 5] = [
        "src/main.rs",
        "src/picker.rs",
        "README.md",
        "docs/guide.md",
        "Cargo.toml",
    ];

    #[test]
    fn filtering_ranks_matches_and_keeps_input_order_for_ties() {
        let mut state = State::new(items(&FILES), true);
        typed(&mut state, "md");
        let shown: Vec<&str> = state.visible_matches().iter().map(|&i| FILES[i]).collect();
        assert_eq!(shown, ["README.md", "docs/guide.md"]);
        state.handle(key(KeyCode::Backspace));
        state.handle(key(KeyCode::Backspace));
        assert_eq!(state.visible_matches(), [0, 1, 2, 3, 4]);
        typed(&mut state, "zzz");
        assert!(state.visible_matches().is_empty());
        assert_eq!(
            state.handle(key(KeyCode::Enter)),
            Step::Done(Outcome::Selected(vec![]))
        );
    }

    #[test]
    fn marks_survive_filtering_and_return_in_input_order() {
        let mut state = State::new(items(&FILES), true);
        typed(&mut state, "picker");
        state.handle(key(KeyCode::Tab));
        state.handle(ctrl('u'));
        // Clearing the query keeps the cursor on the marked item; move to the first one.
        assert_eq!(state.current(), Some(1));
        state.handle(key(KeyCode::Home));
        state.handle(key(KeyCode::Tab));
        assert_eq!(state.marked(), [0, 1]);
        typed(&mut state, "md");
        state.handle(ctrl('a'));
        assert_eq!(state.marked(), [0, 1, 2, 3]);
        state.handle(ctrl('d'));
        assert_eq!(state.marked(), [0, 1]);
        state.handle(ctrl('u'));
        assert_eq!(
            state.handle(key(KeyCode::Enter)),
            Step::Done(Outcome::Selected(vec![0, 1]))
        );
    }

    #[test]
    fn enter_without_marks_takes_the_cursor_item() {
        let mut state = State::new(items(&FILES), true);
        state.handle(key(KeyCode::Down));
        state.handle(key(KeyCode::Down));
        assert_eq!(
            state.handle(key(KeyCode::Enter)),
            Step::Done(Outcome::Selected(vec![2]))
        );
    }

    #[test]
    fn single_mode_ignores_marking() {
        let mut state = State::new(items(&FILES), false);
        state.handle(key(KeyCode::Tab));
        state.handle(ctrl('a'));
        assert!(state.marked().is_empty());
        assert_eq!(
            state.handle(key(KeyCode::Enter)),
            Step::Done(Outcome::Selected(vec![0]))
        );
    }

    #[test]
    fn cancel_keys() {
        for event in [key(KeyCode::Esc), ctrl('c'), ctrl('g'), ctrl('q')] {
            let mut state = State::new(items(&FILES), true);
            state.handle(key(KeyCode::Tab));
            assert_eq!(state.handle(event), Step::Done(Outcome::Cancelled));
        }
    }

    #[test]
    fn cursor_follows_its_item_through_refiltering() {
        let mut state = State::new(items(&FILES), true);
        state.handle(key(KeyCode::Down));
        state.handle(key(KeyCode::Down));
        state.handle(key(KeyCode::Down));
        assert_eq!(state.current(), Some(3));
        typed(&mut state, "md");
        assert_eq!(state.current(), Some(3));
        state.handle(key(KeyCode::End));
        state.handle(key(KeyCode::Down));
        assert_eq!(state.current(), Some(3));
        state.handle(key(KeyCode::Home));
        assert_eq!(state.current(), Some(2));
    }

    #[test]
    fn scrolling_keeps_the_cursor_visible() {
        let names: Vec<String> = (0..50).map(|n| format!("file{n:02}.txt")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut state = State::new(items(&refs), true);
        let options = Options {
            prompt: "Files: ".into(),
            header: vec!["hint".into()],
            multi: true,
        };
        let mut terminal = Terminal::new(TestBackend::new(60, 13)).unwrap();
        let rows = list_height(Rect::new(0, 0, 60, 13), 1) as usize;
        for _ in 0..rows + 4 {
            state.handle(key(KeyCode::Down));
        }
        terminal
            .draw(|f| render(f, &mut state, &options, Some("preview text")))
            .unwrap();
        let screen = format!("{:?}", terminal.backend().buffer());
        assert!(
            screen.contains(&format!("> {}", names[rows + 4])) || screen.contains(&names[rows + 4])
        );
        assert!(!screen.contains("file00.txt"));
        state.handle(key(KeyCode::PageUp));
        state.handle(key(KeyCode::PageUp));
        terminal
            .draw(|f| render(f, &mut state, &options, None))
            .unwrap();
        let screen = format!("{:?}", terminal.backend().buffer());
        assert!(screen.contains("file00.txt"));
    }

    #[test]
    fn renders_prompt_counts_header_marks_and_preview() {
        let mut state = State::new(items(&FILES), true);
        state.handle(key(KeyCode::Tab));
        typed(&mut state, "src");
        let options = Options {
            prompt: "Files: ".into(),
            header: vec!["TAB: multi-select | CTRL-A: select all".into()],
            multi: true,
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 10)).unwrap();
        terminal
            .draw(|f| render(f, &mut state, &options, None))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let row = |y: u16| -> String {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect()
        };
        assert!(row(0).starts_with("Files: src"));
        assert!(row(1).starts_with("  2/5 (1 selected, ~10 tokens)"));
        assert!(row(2).starts_with("TAB: multi-select | CTRL-A: select all"));
        assert!(row(3).starts_with(" + src/main.rs") || row(3).starts_with(">+ src/main.rs"));
        assert!(row(3).contains("src/main.rs"));
        assert!((3..10).any(|y| row(y).contains("Loading preview...")));
    }
}
