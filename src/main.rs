//! Inline ratatui front-end for todo.txt. Pulls the todo repo (git) before opening,
//! commits every change, and pushes on exit.
mod todo;
mod ui;

use std::{
    ffi::OsStr,
    io::{self, Stdout},
    path::{Path, PathBuf},
    process::Command,
};

use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
        terminal::{disable_raw_mode, enable_raw_mode},
    },
    Terminal, TerminalOptions, Viewport,
};

use todo::{Axis, Store};
use ui::Theme;

const HEIGHT: u16 = 17;

/// Byte offset of char position `pos` in `s`; the end of `s` when `pos` is past it.
fn byte_at(s: &str, pos: usize) -> usize {
    s.char_indices().nth(pos).map_or(s.len(), |(b, _)| b)
}

/// What the bottom input line is for.
#[derive(Clone, Copy, PartialEq)]
pub enum Prompt {
    Add,
    /// Line index of the open todo being edited.
    Edit(usize),
    Filter,
}

struct App {
    store: Store,
    /// Git repo holding the todo files, if any.
    repo: Option<PathBuf>,
    /// Open todos.
    lines: Vec<String>,
    /// Completed todos from done.txt, shown after the open ones when `show_done`.
    done: Vec<String>,
    show_done: bool,
    /// Selected value per axis, in `Axis::ALL` order. 0 = All.
    filters: [usize; 3],
    /// Which filter Left/Right change.
    slot: usize,
    /// Free-text filter (`/`): case-insensitive substring of the line.
    query: String,
    /// Index into `rows()`.
    item: usize,
    /// Some(buffer) while a prompt is open.
    input: Option<String>,
    prompt: Prompt,
    /// Cursor position in `input`, in chars.
    cursor: usize,
    status: String,
    theme: Theme,
}

impl App {
    /// Open todos, then done ones when shown. Row indices point into this list.
    fn source(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.lines.iter().map(String::as_str).collect();
        if self.show_done {
            v.extend(self.done.iter().map(String::as_str));
        }
        v
    }

    /// "All" followed by every value on the axis, with line counts.
    fn values(&self, axis: Axis) -> Vec<(String, usize)> {
        let src = self.source();
        let mut v = vec![("All".to_string(), src.len())];
        v.extend(todo::groups(&src[..], axis));
        v
    }

    /// Axes that have at least one real value, in tab order.
    /// "(none)" does not count: every line without a context falls into it.
    fn visible(&self) -> Vec<Axis> {
        let src = self.source();
        Axis::ALL
            .into_iter()
            .filter(|&a| todo::groups(&src[..], a).iter().any(|(k, _)| k != todo::NONE))
            .collect()
    }

    /// The non-All filter selections, as (axis, value key) pairs.
    fn active(&self) -> Vec<(Axis, String)> {
        Axis::ALL
            .iter()
            .enumerate()
            .filter(|(s, _)| self.filters[*s] > 0)
            .map(|(s, &axis)| (axis, self.values(axis)[self.filters[s]].0.clone()))
            .collect()
    }

    /// Indices into `source()` matching every active filter and the text query.
    /// Open todos come first, then done ones, each by priority.
    fn rows(&self) -> Vec<usize> {
        let src = self.source();
        let active = self.active();
        let query = self.query.to_lowercase();
        let open = self.lines.len();
        let mut rows: Vec<usize> = (0..src.len())
            .filter(|&i| active.iter().all(|(axis, k)| todo::matches(src[i], *axis, k)))
            .filter(|&i| query.is_empty() || src[i].to_lowercase().contains(query.as_str()))
            .collect();
        rows.sort_by_key(|&i| (i >= open, todo::priority(src[i]).unwrap_or('~')));
        rows
    }

    fn clamp(&mut self) {
        for (s, &axis) in Axis::ALL.iter().enumerate() {
            let max = self.values(axis).len() - 1;
            self.filters[s] = self.filters[s].min(max);
        }
        self.item = self.item.min(self.rows().len().saturating_sub(1));
        // If the focused axis lost its last value, move focus to a visible one.
        let visible = self.visible();
        if let Some(&first) = visible.first() {
            if !visible.contains(&Axis::ALL[self.slot]) {
                self.slot = Self::slot_of(first);
            }
        }
    }

    fn slot_of(axis: Axis) -> usize {
        Axis::ALL.iter().position(|&a| a == axis).unwrap_or(0)
    }

    /// Writes todo.txt. Returns false and sets the status line on failure.
    fn save(&mut self) -> bool {
        match self.store.save(&self.lines) {
            Ok(()) => true,
            Err(e) => {
                self.status = format!("save failed: {e}");
                false
            }
        }
    }

    /// Writes done.txt. Returns false and sets the status line on failure.
    fn save_done(&mut self) -> bool {
        match self.store.save_done(&self.done) {
            Ok(()) => true,
            Err(e) => {
                self.status = format!("save failed: {e}");
                false
            }
        }
    }

    /// One commit per change, pushed right away. No-op when the todo files are not in a git repo.
    fn commit(&mut self, msg: &str) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        match commit_files(&repo, &[self.store.todo.as_path(), self.store.done.as_path()], msg) {
            Ok(()) => {
                if let Err(e) = git(&repo, &["push"]) {
                    self.status = format!("push failed: {e}");
                }
            }
            Err(e) => self.status = format!("commit failed: {e}"),
        }
    }

    fn add(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.lines.push(text.to_string());
        if self.save() {
            self.commit(&format!("todo: add {text}"));
        }
    }

    /// Replaces open todo `i` with `text`, one commit.
    fn edit(&mut self, i: usize, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.lines[i] = text.to_string();
        if self.save() {
            self.commit(&format!("todo: edit {text}"));
        }
    }

    /// The selected open todo's line index, if the selection is an open todo.
    fn selected_open(&self) -> Option<usize> {
        self.rows()
            .get(self.item)
            .copied()
            .filter(|&i| i < self.lines.len())
    }

    fn start_prompt(&mut self, text: String, prompt: Prompt) {
        self.cursor = text.chars().count();
        self.input = Some(text);
        self.prompt = prompt;
    }

    /// `x`: complete the selected open todo, or restore the selected done one.
    fn toggle_selected(&mut self) {
        let Some(&i) = self.rows().get(self.item) else {
            return;
        };
        if i < self.lines.len() {
            self.complete(i);
        } else {
            self.restore(i - self.lines.len());
        }
    }

    /// Moves done item `d` back to todo.txt, one commit.
    fn restore(&mut self, d: usize) {
        let line = todo::body(&self.done.remove(d)).to_string();
        self.lines.push(line.clone());
        if self.save() && self.save_done() {
            self.commit(&format!("todo: restore {line}"));
        }
        self.clamp();
    }

    fn complete(&mut self, i: usize) {
        let line = self.lines[i].clone();
        let done = format!("x {} {line}", todo::today());
        if let Err(e) = self.store.append_done(&done) {
            self.status = format!("done.txt write failed: {e}");
            return;
        }
        self.done.push(done);
        self.lines.remove(i);
        if self.save() {
            self.commit(&format!("todo: done {line}"));
        }
        self.clamp();
    }

    fn toggle_done(&mut self) {
        self.show_done = !self.show_done;
        self.item = 0;
    }

    fn move_item(&mut self, delta: isize) {
        let max = self.rows().len().saturating_sub(1) as isize;
        self.item = (self.item as isize + delta).clamp(0, max) as usize;
    }

    /// Left/Right: step the focused filter's value.
    fn cycle_value(&mut self, delta: isize) {
        let max = (self.values(Axis::ALL[self.slot]).len() - 1) as isize;
        self.filters[self.slot] = (self.filters[self.slot] as isize + delta).clamp(0, max) as usize;
        self.item = 0;
    }

    /// Tab: focus the next/previous visible filter.
    fn move_slot(&mut self, delta: isize) {
        let visible = self.visible();
        if visible.is_empty() {
            return;
        }
        let cur = visible
            .iter()
            .position(|&a| a == Axis::ALL[self.slot])
            .unwrap_or(0) as isize;
        let next = (cur + delta).rem_euclid(visible.len() as isize) as usize;
        self.slot = Self::slot_of(visible[next]);
    }

    /// Keys while a prompt is open. The filter follows the buffer as you type.
    fn on_prompt_key(&mut self, key: KeyEvent) {
        let Some(buf) = self.input.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let at = byte_at(buf, self.cursor);
                buf.insert(at, c);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let at = byte_at(buf, self.cursor);
                    buf.remove(at);
                }
            }
            KeyCode::Delete => {
                if self.cursor < buf.chars().count() {
                    let at = byte_at(buf, self.cursor);
                    buf.remove(at);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(buf.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = buf.chars().count(),
            KeyCode::Esc => {
                self.input = None;
                if self.prompt == Prompt::Filter {
                    self.query.clear();
                }
            }
            KeyCode::Enter => {
                let text = self.input.take().unwrap_or_default();
                match self.prompt {
                    Prompt::Add => self.add(&text),
                    Prompt::Edit(i) => self.edit(i, &text),
                    Prompt::Filter => self.query = text.trim().to_string(),
                }
            }
            _ => {}
        }
        if self.prompt == Prompt::Filter {
            if let Some(buf) = &self.input {
                self.query = buf.clone();
            }
        }
        self.item = 0;
    }

    /// Returns true when the app should quit.
    fn on_key(&mut self, key: KeyEvent) -> bool {
        if self.input.is_some() {
            self.on_prompt_key(key);
            return false;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => return true,
            KeyCode::Char('q') => return true,
            // Esc clears an active filter first, and quits otherwise.
            KeyCode::Esc => {
                if self.query.is_empty() {
                    return true;
                }
                self.query.clear();
                self.item = 0;
            }
            KeyCode::Char('/') => self.start_prompt(self.query.clone(), Prompt::Filter),
            KeyCode::Char('a') => self.start_prompt(String::new(), Prompt::Add),
            KeyCode::Char('e') => {
                if let Some(i) = self.selected_open() {
                    self.start_prompt(self.lines[i].clone(), Prompt::Edit(i));
                }
            }
            KeyCode::Char('x') => self.toggle_selected(),
            KeyCode::Char('d') => self.toggle_done(),
            KeyCode::Tab => self.move_slot(1),
            KeyCode::BackTab => self.move_slot(-1),
            KeyCode::Up | KeyCode::Char('k') => self.move_item(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_item(1),
            KeyCode::Left | KeyCode::Char('h') => self.cycle_value(-1),
            KeyCode::Right | KeyCode::Char('l') => self.cycle_value(1),
            _ => {}
        }
        false
    }
}

fn git<S: AsRef<OsStr>>(repo: &Path, args: &[S]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Stages the existing `files` and commits them. No-op when they have no changes.
fn commit_files(repo: &Path, files: &[&Path], msg: &str) -> Result<(), String> {
    let existing: Vec<&OsStr> = files
        .iter()
        .filter(|f| f.exists())
        .map(|f| f.as_os_str())
        .collect();
    if existing.is_empty() {
        return Ok(());
    }
    let mut status: Vec<&OsStr> = vec!["status".as_ref(), "--porcelain".as_ref(), "--".as_ref()];
    status.extend(&existing);
    if git(repo, &status)?.trim().is_empty() {
        return Ok(());
    }
    let mut add: Vec<&OsStr> = vec!["add".as_ref(), "--".as_ref()];
    add.extend(&existing);
    git(repo, &add)?;
    git(repo, &["commit", "-m", msg])?;
    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                // Errors show until the next key press.
                app.status.clear();
                if app.on_key(key) {
                    return Ok(());
                }
                app.clamp();
            }
        }
    }
}

fn main() -> io::Result<()> {
    let store = Store::locate();
    let repo = store
        .todo
        .parent()
        .map(Path::to_path_buf)
        .filter(|d| d.join(".git").exists());

    let mut status = String::new();
    if let Some(repo) = &repo {
        if let Err(e) = git(repo, &["pull", "--rebase", "--autostash"]) {
            status = format!("pull failed: {e}");
        }
    }
    let lines = store.load()?;
    let done = store.load_done()?;
    let mut app = App {
        store,
        repo,
        lines,
        done,
        show_done: false,
        filters: [0; 3],
        slot: 0,
        query: String::new(),
        item: 0,
        input: None,
        prompt: Prompt::Add,
        cursor: 0,
        status,
        theme: Theme::nord(),
    };
    app.clamp();

    enable_raw_mode()?;
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: Viewport::Inline(HEIGHT),
        },
    )?;
    let result = run(&mut terminal, &mut app);
    // Erase the inline viewport so the shell prompt returns where the table was.
    terminal.clear()?;
    disable_raw_mode()?;
    result
}
