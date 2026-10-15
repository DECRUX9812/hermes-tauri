//! The `cmd.exe` command line for a `.cmd`/`.bat` backend (Windows).
//!
//! `CreateProcessW` cannot start a batch file. It has an undocumented feature
//! that turns a `.bat` program into `cmd.exe /c` plus the batch file, and Rust's
//! `Command` "preserves this behavior ... for historical reasons" while warning
//! that "running batch scripts in this way may be removed in the future and so
//! should not be relied upon" (<https://doc.rust-lang.org/std/process/index.html#batch-file-special-handling>).
//! `hermes-shell-config` resolves `hermes.cmd` on `PATH`, so the shell would be
//! relying on exactly that. Instead the spawn path asks for `cmd.exe` itself and
//! builds the command line here, which also makes the quoting ours to test.
//!
//! The quoting mirrors `std::sys::args::windows::make_bat_command_line`, the
//! construction reviewed for CVE-2024-24576: the whole invocation is wrapped in
//! an extra pair of quotes that `cmd.exe` strips again, `"` is escaped by
//! doubling it, and `%` is neutralised so that no `%VAR%` is expanded.
//! Arguments that cannot be passed through `cmd.exe` safely are refused instead
//! of being silently mangled.

use std::io;
use std::path::Path;

/// Switches `cmd.exe` is started with, in order.
///
/// `/e:ON` enables command extensions, which the `%` neutralisation relies on;
/// `/v:OFF` keeps delayed expansion off so `!` stays literal; `/d` skips any
/// AutoRun command; `/c` runs the command and exits.
pub(crate) const CMD_EXE_SWITCHES: [&str; 4] = ["/e:ON", "/v:OFF", "/d", "/c"];

/// True when `program` is a batch file (`.cmd` or `.bat`, case-insensitive).
pub(crate) fn is_batch(program: &str) -> bool {
    Path::new(program).extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
    })
}

/// The literal command line to append after the switches in [`CMD_EXE_SWITCHES`],
/// ready for `CommandExt::raw_arg`.
///
/// It starts with two quotes: the first one pairs with the trailing quote and
/// makes `cmd.exe` strip them again, leaving `"program" args`.
pub(crate) fn cmd_exe_command_line(program: &str, args: &[String]) -> io::Result<String> {
    // Windows file names cannot contain `"` or end with `\`, so either would
    // only make `cmd.exe` read a different command.
    if program.contains('"') || program.ends_with('\\') {
        return Err(invalid(format!(
            "the program path `{program}` contains `\"` or ends with `\\`"
        )));
    }
    let mut line = String::with_capacity(program.len() + 2);
    line.push('"');
    line.push('"');
    line.push_str(program);
    line.push('"');
    for arg in args {
        if arg.contains(['\r', '\n']) {
            return Err(invalid(
                "an argument contains a line break, which cmd.exe would read as a command separator",
            ));
        }
        line.push(' ');
        push_bat_arg(&mut line, arg);
    }
    line.push('"');
    Ok(line)
}

/// Appends one argument the way `cmd.exe` reads it back.
fn push_bat_arg(line: &mut String, arg: &str) {
    // Rather than listing every character `cmd.exe` treats specially, quote
    // unless every character is known to be harmless. An empty argument needs
    // the quotes to survive at all, and a trailing `\` needs them because it
    // would otherwise escape the closing quote of a `"%~1"` in the batch file.
    const SAFE: &str = r"#$*+-./:?@\_";
    let quote = arg.is_empty()
        || arg.ends_with('\\')
        || arg.chars().any(|c| {
            (c.is_ascii() && !(c.is_ascii_alphanumeric() || SAFE.contains(c))) || c.is_control()
        });

    if quote {
        line.push('"');
    }
    let mut backslashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            backslashes += 1;
        } else {
            match c {
                // `cmd.exe` escapes `"` by doubling it. The backslashes stay as
                // they are: they are not escape characters for `cmd.exe`.
                '"' => {
                    push_backslashes(line, backslashes);
                    line.push('"');
                }
                // `%VAR%` expands even inside quotes. `%%cd:~,%` expands to
                // nothing (a zero-length substring of the built-in `cd`) and
                // stops the following `%` from starting a variable name.
                '%' => line.push_str("%%cd:~,"),
                _ => {}
            }
            backslashes = 0;
        }
        line.push(c);
    }
    if quote {
        push_backslashes(line, backslashes);
        line.push('"');
    }
}

fn push_backslashes(line: &mut String, count: usize) {
    for _ in 0..count {
        line.push('\\');
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(program: &str, args: &[&str]) -> String {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        cmd_exe_command_line(program, &args).expect("builds a command line")
    }

    #[test]
    fn recognises_batch_programs_case_insensitively() {
        for program in [
            "hermes.cmd",
            "HERMES.CMD",
            "Hermes.Bat",
            r"C:\Program Files\Hermes\hermes.cmd",
            r"C:\hermes.bat",
        ] {
            assert!(is_batch(program), "{program} should be a batch file");
        }
        for program in [
            "hermes",
            "hermes.exe",
            "hermes.cmd.exe",
            "hermes.cmdx",
            "hermes.bat.txt",
            r"C:\Program Files\Hermes\hermes.exe",
            "",
        ] {
            assert!(!is_batch(program), "{program} should not be a batch file");
        }
    }

    #[test]
    fn the_switch_set_is_the_reviewed_one() {
        // The `%` neutralisation below depends on `/e:ON`, and `/v:OFF` keeps
        // `!` literal; changing these means re-checking the quoting.
        assert_eq!(CMD_EXE_SWITCHES, ["/e:ON", "/v:OFF", "/d", "/c"]);
    }

    #[test]
    fn wraps_the_program_and_its_arguments_in_one_extra_quote_pair() {
        // cmd.exe strips the first and last quote of the `/c` string, which
        // leaves `"program" args` behind.
        assert_eq!(
            line(
                r"C:\Program Files\Hermes\hermes.cmd",
                &["webapp", "--isolated"]
            ),
            "\"\"C:\\Program Files\\Hermes\\hermes.cmd\" webapp --isolated\""
        );
    }

    #[test]
    fn a_program_without_arguments_still_gets_the_extra_quote_pair() {
        assert_eq!(line("hermes.cmd", &[]), "\"\"hermes.cmd\"\"");
    }

    #[test]
    fn quotes_what_cmd_exe_would_read_differently_unquoted() {
        // A space, an empty argument, shell metacharacters and a control char.
        assert_eq!(line("hermes.cmd", &["a b"]), "\"\"hermes.cmd\" \"a b\"\"");
        assert_eq!(line("hermes.cmd", &[""]), "\"\"hermes.cmd\" \"\"\"");
        assert_eq!(
            line("hermes.cmd", &["a&b|c>d"]),
            "\"\"hermes.cmd\" \"a&b|c>d\"\""
        );
        assert_eq!(
            line("hermes.cmd", &["a\u{7}b"]),
            "\"\"hermes.cmd\" \"a\u{7}b\"\""
        );
    }

    #[test]
    fn leaves_harmless_arguments_alone() {
        assert_eq!(
            line(
                "hermes.cmd",
                &[
                    "--port",
                    "0",
                    "127.0.0.1",
                    r"C:\Program Files\Hermes",
                    "a_b*c"
                ]
            ),
            "\"\"hermes.cmd\" --port 0 127.0.0.1 \"C:\\Program Files\\Hermes\" a_b*c\""
        );
    }

    #[test]
    fn doubles_quotes_and_keeps_backslashes_for_cmd_exe() {
        assert_eq!(
            line("hermes.cmd", &[r#"a"b"#]),
            "\"\"hermes.cmd\" \"a\"\"b\"\""
        );
        assert_eq!(
            line("hermes.cmd", &[r#"a\"b"#]),
            "\"\"hermes.cmd\" \"a\\\\\"\"b\"\""
        );
        // A trailing backslash is doubled so it cannot escape the closing quote.
        assert_eq!(
            line("hermes.cmd", &[r"C:\dir\"]),
            "\"\"hermes.cmd\" \"C:\\dir\\\\\"\""
        );
    }

    #[test]
    fn neutralises_percent_so_no_variable_expands() {
        // Each `%` becomes `%%cd:~,%`: `%cd:~,%` is a zero-length substring of
        // the built-in `cd`, so nothing is inserted and no variable name starts.
        assert_eq!(
            line("hermes.cmd", &["%PATH%", "100%"]),
            "\"\"hermes.cmd\" \"%%cd:~,%PATH%%cd:~,%\" \"100%%cd:~,%\"\""
        );
    }

    #[test]
    fn refuses_what_cmd_exe_cannot_be_given_safely() {
        let line_break = cmd_exe_command_line("hermes.cmd", &["a\nb".to_owned()])
            .expect_err("a line break is refused");
        assert_eq!(line_break.kind(), io::ErrorKind::InvalidInput);
        assert!(
            cmd_exe_command_line("hermes.cmd", &["a\rb".to_owned()]).is_err(),
            "a carriage return is refused"
        );
        assert!(
            cmd_exe_command_line("hermes\".cmd", &[]).is_err(),
            "a quote in the program path is refused"
        );
        assert!(
            cmd_exe_command_line("hermes.cmd\\", &[]).is_err(),
            "a program path ending in a backslash is refused"
        );
    }
}
