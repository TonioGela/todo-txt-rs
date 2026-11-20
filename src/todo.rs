//! todo.txt model and file storage.
use std::{
    collections::{BTreeMap, HashMap},
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Group key for lines that have no value on an axis.
pub const NONE: &str = "(none)";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    Project,
    Context,
    Priority,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::Project, Axis::Context, Axis::Priority];

    pub fn title(self) -> &'static str {
        match self {
            Axis::Project => "Project",
            Axis::Context => "Context",
            Axis::Priority => "Priority",
        }
    }

    /// Group keys a line belongs to on this axis.
    pub fn keys(self, line: &str) -> Vec<String> {
        let keys: Vec<String> = match self {
            Axis::Project => tokens(line, '+').collect(),
            Axis::Context => tokens(line, '@').collect(),
            Axis::Priority => priority(line).map(|p| format!("({p})")).into_iter().collect(),
        };
        if keys.is_empty() {
            vec![NONE.to_string()]
        } else {
            keys
        }
    }
}

fn tokens<'a>(line: &'a str, sigil: char) -> impl Iterator<Item = String> + 'a {
    line.split_whitespace()
        .filter(move |t| t.starts_with(sigil) && t.len() > 1)
        .map(String::from)
}

/// The todo text of a line, without a done line's "x " and completion date.
pub fn body(line: &str) -> &str {
    let Some(rest) = line.strip_prefix("x ") else {
        return line;
    };
    let date = rest.as_bytes();
    let is_date = date.len() >= 11
        && date[10] == b' '
        && date[..10].iter().all(|b| b.is_ascii_digit() || *b == b'-');
    if is_date {
        &rest[11..]
    } else {
        rest
    }
}

/// Priority of a line: "(A) rest of the line" -> 'A'. Done lines count too.
pub fn priority(line: &str) -> Option<char> {
    match body(line).as_bytes() {
        [b'(', p @ b'A'..=b'Z', b')', b' ', ..] => Some(*p as char),
        _ => None,
    }
}

/// Distinct keys on an axis with how many lines carry each. `NONE` sorts last.
pub fn groups<S: AsRef<str>>(lines: &[S], axis: Axis) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for line in lines {
        for key in axis.keys(line.as_ref()) {
            *counts.entry(key).or_default() += 1;
        }
    }
    let (none, mut rest): (Vec<_>, Vec<_>) = counts.into_iter().partition(|(k, _)| k == NONE);
    rest.extend(none);
    rest
}

pub fn matches(line: &str, axis: Axis, key: &str) -> bool {
    axis.keys(line).iter().any(|k| k == key)
}

/// Today's date as YYYY-MM-DD, in UTC.
pub fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    civil_date((secs / 86_400) as i64)
}

/// Date of `days` since 1970-01-01 (Howard Hinnant's civil_from_days).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Parses todo.sh's config: shell-style `KEY=value` or `export KEY=value` lines, `#` comments.
pub fn parse_config(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (k, v) = line.split_once('=')?;
            let k = k.trim();
            if k.is_empty() || k.starts_with('#') {
                return None;
            }
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

/// Where the todo files live. Precedence: env vars, then `~/.todo/config` or `~/.todo.cfg`
/// (todo.sh's config files), then `~/.todo` (todo.sh's default dir).
pub struct Store {
    pub todo: PathBuf,
    pub done: PathBuf,
}

impl Store {
    pub fn locate() -> Store {
        let home = PathBuf::from(env::var_os("HOME").unwrap_or_default());
        let text = [home.join(".todo/config"), home.join(".todo.cfg")]
            .iter()
            .find_map(|p| fs::read_to_string(p).ok())
            .unwrap_or_default();
        let cfg = parse_config(&text);
        let get = |k: &str| env::var(k).ok().or_else(|| cfg.get(k).cloned()).map(PathBuf::from);

        let dir = get("TODO_DIR").unwrap_or_else(|| home.join(".todo"));
        Store {
            todo: get("TODO_FILE").unwrap_or_else(|| dir.join("todo.txt")),
            done: get("DONE_FILE").unwrap_or_else(|| dir.join("done.txt")),
        }
    }

    pub fn load(&self) -> io::Result<Vec<String>> {
        read_lines(&self.todo)
    }

    pub fn load_done(&self) -> io::Result<Vec<String>> {
        read_lines(&self.done)
    }

    pub fn save(&self, lines: &[String]) -> io::Result<()> {
        write_lines(&self.todo, lines)
    }

    pub fn save_done(&self, lines: &[String]) -> io::Result<()> {
        write_lines(&self.done, lines)
    }

    pub fn append_done(&self, line: &str) -> io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.done)?;
        writeln!(f, "{line}")
    }
}

fn write_lines(path: &Path, lines: &[String]) -> io::Result<()> {
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    fs::write(path, out)
}

/// Non-blank lines of a file; a missing file is empty.
fn read_lines(path: &Path) -> io::Result<Vec<String>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(s
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(String::from)
            .collect()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_axes() {
        let l = "(B) call +Home @phone @out";
        assert_eq!(priority(l), Some('B'));
        assert_eq!(Axis::Project.keys(l), ["+Home"]);
        assert_eq!(Axis::Context.keys(l), ["@phone", "@out"]);
        assert_eq!(Axis::Priority.keys("buy milk"), [NONE]);
        assert_eq!(priority("(a) lower"), None);
        assert_eq!(priority("(A)no space"), None);
    }

    #[test]
    fn done_lines_keep_their_priority() {
        assert_eq!(priority("x 2026-11-20 (A) sistema +Nix"), Some('A'));
        assert_eq!(priority("x 2026-11-20 buy milk"), None);
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(10_957), "2000-01-01");
        assert_eq!(civil_date(10_957 + 59), "2000-02-29"); // 2000 is a leap year
        assert_eq!(civil_date(10_957 + 60), "2000-03-01");
    }

    #[test]
    fn parses_todo_sh_config() {
        let cfg = parse_config("# comment\nexport TODO_DIR=\"/x/y\"\nTODO_FILE='/a/t.txt'\n");
        assert_eq!(cfg["TODO_DIR"], "/x/y");
        assert_eq!(cfg["TODO_FILE"], "/a/t.txt");
        assert!(!cfg.contains_key("# comment"));
    }
}
