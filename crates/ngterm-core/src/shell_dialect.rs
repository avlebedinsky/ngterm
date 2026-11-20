//! A captured argv as one line typed at a shell's prompt, quoted for that shell: what a rerun restore types into a
//! pane's login shell (upstream's `LaunchSeed` types `CommandRestore.shellQuotedLine(argv) + "\n"`, a POSIX line,
//! whatever the shell; a Rust-only extension for the shells a Windows pane runs), and the PowerShell call a live
//! replay runs ([`crate::live::replay`]).
//!
//! The dialect is the pane's shell's, read off its program's name ([`ShellDialect::of_shell`], past an `env` that
//! runs it), a Windows `.exe` dropped in any case:
//!
//! - **POSIX** (`sh`, `bash`, `zsh`, `dash`, `ksh` and kin, `tcsh`, `csh`): upstream's line, each word single-quoted
//!   ([`shell_quoted_line`]), a line feed after it. Git for Windows' `sh` and `bash` are POSIX shells too.
//! - **fish**: each word single-quoted the way fish reads it, where a backslash escapes another backslash or a quote
//!   ([`fish_quoted`]); a POSIX word with a backslash before its closing quote would not end there.
//! - **PowerShell** (7's `pwsh`, Windows PowerShell 5.1's `powershell`): the call operator and each word single-quoted
//!   (`''` doubling, [`powershell_quoted`]), a carriage return after it, which `PSReadLine` takes for Enter (a line
//!   feed is Ctrl+J there): `& 'program' 'argument'`. The call operator always, as a bare first word can be a keyword
//!   or a number. A native program receives its arguments through PowerShell's argument passing, which differs by
//!   version and preference (`$PSNativeCommandArgumentPassing`): 5.1 and 7.0–7.2 have only the legacy one, which drops
//!   an empty argument and loses an embedded double quote. So an argv any of the passings would change runs in a
//!   scope of its own under `Legacy`, each argument already quoted for the C runtime's split ([`crt_quoted`]): legacy
//!   passes such a word as it is (its whitespace sits inside its quotes), so the program's split rebuilds the argv on
//!   every version and OS (.NET's own split on Linux and macOS follows the same rules). An argv of plain words passes
//!   the same under every passing and is typed without that scope. Exact for programs that split their command line
//!   as the C runtime does (`CommandLineToArgvW`); `cmd.exe`, batch files and `msiexec` parse theirs their own way. A
//!   bare program name runs what PowerShell finds for it first, an alias or a function before a program, as it did
//!   when typed.
//! - **cmd** (`cmd.exe`): the argv quoted for the C runtime's split, then escaped for `cmd.exe`'s own reading of a
//!   prompt line, a carriage return after it. `cmd.exe` toggles its quoted state at every `"`, and only outside quotes
//!   does a caret escape, so every quote of the arguments is typed as `^"` and `cmd.exe` never enters its quoted state
//!   there: each of `^ & | < > ( ) !` is caret-escaped, and each `%` is followed by a caret, which keeps `%NAME%` from
//!   naming a variable (the prompt has no `%%`). The program is typed bare when it is a plain path, else in real
//!   quotes; one holding a `%` or a `"` is refused, as no quoting keeps it whole. Limits: a program reading its
//!   command line as the C runtime does gets the exact argv; a batch file or an internal command of the same name as
//!   the program (`echo`, `start`) does not; with delayed expansion turned on (`cmd /V:ON`, or the registry's
//!   `DelayedExpansion`) a `!` loses its caret and a `!NAME!` expands; a line over 8191 characters is refused by
//!   `cmd.exe` itself.
//! - **Unknown** (`nu`, `xonsh`, `elvish`, a config `command` naming another program): the safest choice is typing
//!   only what every dialect reads alike: plain words (ASCII letters and digits, `_ - . / : +`, on Windows `\` too,
//!   none empty), bare, with a carriage return, Enter's own key code. An argv with any other word is not typed: the
//!   pane comes up as a plain shell, as for a denylisted program, rather than one whose line a shell could run
//!   differently.

use crate::command_restore::{basename, shell_name, shell_quoted_line};

/// The POSIX-family shells, by name: upstream's and their kin (the Korn shells' `ksh93`, OpenBSD's ports `oksh` and
/// `loksh`, `mksh`'s legacy `lksh`, restricted `rbash`, Debian's `posh`, the Schily `bosh`, `BusyBox`'s `ash` and
/// `hush`).
const POSIX_SHELLS: [&str; 20] = [
    "sh", "bash", "rbash", "zsh", "dash", "ash", "hush", "ksh", "ksh93", "mksh", "lksh", "oksh", "loksh", "pdksh",
    "posh", "bosh", "yash", "busybox", "tcsh", "csh",
];

/// Whether `word` names `env` (by any path, a Windows `.exe` dropped in any case).
fn is_env(word: &str) -> bool {
    shell_name(basename(word)).eq_ignore_ascii_case("env")
}

/// The program that `env` runs after its options, assignments, split strings and nested wrappers.
fn env_program(arguments: &str) -> Option<&str> {
    let mut words = std::collections::VecDeque::from(env_words(arguments)?);
    while let Some(word) = words.pop_front() {
        let word = unquote(word);
        if word == "-" || (word.contains('=') && !word.starts_with('-')) {
            continue;
        }
        if word == "--" {
            let mut program = unquote(words.pop_front()?);
            if program == "-" {
                program = unquote(words.pop_front()?);
            }
            while program.contains('=') {
                program = unquote(words.pop_front()?);
            }
            if !is_env(program) {
                return Some(program);
            }
        } else if word.starts_with('-') {
            env_option(word, &mut words)?;
        } else if !is_env(word) {
            return Some(word);
        }
    }
    None
}

fn env_option<'a>(word: &'a str, words: &mut std::collections::VecDeque<&'a str>) -> Option<()> {
    let split = if let Some(long) = word.strip_prefix("--") {
        if ["unset", "chdir"].contains(&long) {
            words.pop_front()?;
        }
        if long == "split-string" {
            Some(words.pop_front()?)
        } else {
            long.strip_prefix("split-string=")
        }
    } else {
        let (takes_next, split) = env_short_options(word.strip_prefix('-')?);
        if takes_next {
            words.pop_front()?;
        }
        match split {
            Some("") => Some(words.pop_front()?),
            split => split,
        }
    };
    if let Some(split) = split {
        // The shell and `env -S` can each change an escape before the program name.
        if split.contains('\\') {
            return None;
        }
        for part in env_words(unquote(split))?.into_iter().rev() {
            words.push_front(part);
        }
    }
    Some(())
}

fn unquote(word: &str) -> &str {
    for quote in ['\'', '"'] {
        if let Some(inner) = word.strip_prefix(quote).and_then(|rest| rest.strip_suffix(quote)) {
            return inner;
        }
    }
    word
}

fn env_words(mut arguments: &str) -> Option<Vec<&str>> {
    let mut words = Vec::new();
    loop {
        arguments = arguments.trim_start();
        if arguments.is_empty() {
            return Some(words);
        }
        let end = env_word_end(arguments)?;
        words.push(&arguments[..end]);
        arguments = &arguments[end..];
    }
}

fn env_word_end(arguments: &str) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    for (at, character) in arguments.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, character) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None | Some('"'), '\\') => escaped = true,
            (None, character) if character.is_whitespace() => return Some(at),
            _ => {}
        }
    }
    (quote.is_none() && !escaped).then_some(arguments.len())
}

/// What a bundle of `env`'s short options (`-iu`, `-uNAME`, `-Szsh`, without the `-`) leaves: whether the next word
/// is an option's value, and the start of `-S`'s words joined to it.
fn env_short_options(bundle: &str) -> (bool, Option<&str>) {
    /// The short options taking a value.
    const VALUED: [char; 3] = ['u', 'C', 'P'];
    for (at, option) in bundle.char_indices() {
        let rest = &bundle[at + option.len_utf8()..];
        if VALUED.contains(&option) {
            return (rest.is_empty(), None);
        }
        if option == 'S' {
            return (false, Some(rest));
        }
    }
    (false, None)
}

/// What `cmd.exe` reads specially outside quotes, a caret escaping each; `%` and `"` have rules of their own.
const CMD_METACHARACTERS: [char; 7] = ['^', '&', '|', '<', '>', '(', ')'];

/// How a shell reads a typed line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellDialect {
    Posix,
    Fish,
    PowerShell,
    Cmd,
    Unknown,
}

impl ShellDialect {
    /// The dialect of the shell program `name` (a path or a bare name, a login `-` and a Windows `.exe` dropped).
    #[must_use]
    pub fn of_program(name: &str) -> Self {
        let name = basename(name.strip_prefix('-').unwrap_or(name));
        let name = shell_name(name).to_ascii_lowercase();
        match name.as_str() {
            "fish" => Self::Fish,
            "pwsh" | "powershell" => Self::PowerShell,
            "cmd" => Self::Cmd,
            posix if POSIX_SHELLS.contains(&posix) => Self::Posix,
            _ => Self::Unknown,
        }
    }

    /// The dialect of a pane's shell as its config names it: a program path that may hold spaces
    /// (`C:\Program Files\PowerShell\7\pwsh.exe`) or a program and its arguments (`pwsh -NoLogo`, `/usr/bin/fish -l`).
    /// The program is the value's shortest run of leading words that names a known shell or `env`, so a path with
    /// spaces is read whole and a shell's argument never stands for it (`bash --rcfile /etc/env`,
    /// `pwsh -File C:\scripts\bash`); [`Self::Unknown`] when no run does. A program `env` is the program it runs
    /// ([`env_program`]: `/usr/bin/env zsh`, `env -S 'zsh -l'`, `C:\Program Files\Git\usr\bin\env.exe bash`).
    #[must_use]
    pub fn of_shell(shell: &str) -> Self {
        shell_program(shell).map_or(Self::Unknown, Self::of_program)
    }

    /// What submits a typed line: upstream's line feed for POSIX shells and fish, else a carriage return, the Enter
    /// key's own code.
    #[must_use]
    pub fn enter(self) -> &'static str {
        match self {
            Self::Posix | Self::Fish => "\n",
            Self::PowerShell | Self::Cmd | Self::Unknown => "\r",
        }
    }

    /// `argv` as one line this dialect runs with exactly those words, without its Enter; `None` when it cannot be
    /// typed safely (module docs) or is empty.
    #[must_use]
    pub fn line<S: AsRef<str>>(self, argv: &[S]) -> Option<String> {
        let (program, arguments) = argv.split_first()?;
        let program = program.as_ref();
        match self {
            Self::Posix => Some(shell_quoted_line(argv)),
            Self::Fish => Some(words(argv, fish_quoted)),
            Self::PowerShell => Some(powershell_call(argv)),
            Self::Cmd => cmd_line(program, arguments),
            Self::Unknown => argv
                .iter()
                .all(|word| is_portable(word.as_ref()))
                .then(|| words(argv, str::to_owned)),
        }
    }

    /// [`Self::line`] and its [`Self::enter`].
    #[must_use]
    pub fn typed<S: AsRef<str>>(self, argv: &[S]) -> Option<String> {
        self.line(argv).map(|line| line + self.enter())
    }
}

/// The shell program after the same path and `env` rules as [`ShellDialect::of_shell`].
pub(crate) fn shell_program(shell: &str) -> Option<&str> {
    let shell = shell.trim_start();
    let mut end = 0;
    loop {
        let rest = &shell[end..];
        let word = rest.trim_start();
        if word.is_empty() {
            return None;
        }
        end += rest.len() - word.len() + word.find(char::is_whitespace).unwrap_or(word.len());
        let program = &shell[..end];
        if is_env(program) {
            return env_program(&shell[end..]);
        }
        if ShellDialect::of_program(program) != ShellDialect::Unknown {
            return Some(program);
        }
    }
}

fn words<S: AsRef<str>>(argv: &[S], quote: impl Fn(&str) -> String) -> String {
    argv.iter()
        .map(|word| quote(word.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `word` reads alike in every dialect, bare.
fn is_portable(word: &str) -> bool {
    !word.is_empty()
        && word.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '+') || (cfg!(windows) && c == '\\')
        })
}

/// `value` as one PowerShell word: a single-quoted string, inside which a single quote is written twice. PowerShell
/// takes the typographic single quotes (U+2018–U+201B) for one as well, so those are doubled too.
#[must_use]
pub fn powershell_quoted(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for character in value.chars() {
        if matches!(character, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}') {
            quoted.push(character);
        }
        quoted.push(character);
    }
    quoted.push('\'');
    quoted
}

/// `value` as one fish word: single quotes, inside which only a backslash and a single quote are escaped.
#[must_use]
pub fn fish_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\\', r"\\").replace('\'', r"\'"))
}

/// The PowerShell statement that runs `argv` as a native program with exactly those arguments (module docs): `& 'p'
/// 'a'`, or, when an argument needs quoting for the C runtime's split, `& { $PSNativeCommandArgumentPassing =
/// 'Legacy' ; & 'p' '"a b"' }`. Its own scope keeps the user's preference as it was.
#[must_use]
pub fn powershell_call<S: AsRef<str>>(argv: &[S]) -> String {
    let Some((program, arguments)) = argv.split_first() else {
        return String::new();
    };
    let encoded: Vec<String> = arguments.iter().map(|word| crt_quoted(word.as_ref())).collect();
    let mut call = format!("& {}", powershell_quoted(program.as_ref()));
    for word in &encoded {
        call.push(' ');
        call.push_str(&powershell_quoted(word));
    }
    let plain = encoded
        .iter()
        .zip(arguments)
        .all(|(word, argument)| word == argument.as_ref());
    if plain {
        call
    } else {
        format!("& {{ $PSNativeCommandArgumentPassing = 'Legacy' ; {call} }}")
    }
}

/// `value` as one word of a command line the C runtime splits (`CommandLineToArgvW`): as it is when it is not empty and
/// holds no space, tab or double quote; else in double quotes, each inner quote and the backslashes before it
/// escaped, the trailing backslashes doubled.
#[must_use]
pub fn crt_quoted(value: &str) -> String {
    if !value.is_empty() && !value.contains([' ', '\t', '"']) {
        return value.to_owned();
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in value.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// The `cmd.exe` prompt line of `program` and `arguments` (module docs); `None` when the program holds a `%` or a `"`.
fn cmd_line<S: AsRef<str>>(program: &str, arguments: &[S]) -> Option<String> {
    if program.is_empty() || program.contains(['%', '"']) {
        return None;
    }
    let bare = program.starts_with(|c: char| c.is_ascii_alphanumeric())
        && program
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '\\' | ':'));
    let mut line = if bare {
        program.to_owned()
    } else {
        format!("\"{program}\"")
    };
    let mut after_percent = false;
    for word in arguments {
        let encoded = format!(" {}", crt_quoted(word.as_ref()));
        for character in encoded.chars() {
            let escaped = character == '"' || character == '!' || CMD_METACHARACTERS.contains(&character);
            if escaped || after_percent {
                line.push('^');
            }
            line.push(character);
            after_percent = character == '%';
        }
    }
    Some(line)
}

#[cfg(test)]
mod tests;
