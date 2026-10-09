mod core;
mod content_index;
mod display;
mod gui;
mod platform;
mod tui;

use std::io::{IsTerminal, stdout};

use anyhow::{Context, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Auto,
    Gui,
    Tui,
}

fn main() -> anyhow::Result<()> {
    let mode = parse_mode()?;
    let home = dirs::home_dir().context("Could not find the user home directory")?;
    let has_terminal = std::io::stdin().is_terminal() && stdout().is_terminal();
    let has_gui = platform::has_graphical_session();
    match select_mode(mode, has_gui, has_terminal)? {
        Mode::Gui => gui::run(home),
        Mode::Tui => tui::run(home),
        Mode::Auto => unreachable!(),
    }
}

fn select_mode(mode: Mode, has_gui: bool, has_terminal: bool) -> anyhow::Result<Mode> {
    let selected = match mode {
        Mode::Gui if has_gui => Mode::Gui,
        Mode::Gui => {
            bail!("No graphical session is available. Use --tui in an interactive terminal.")
        }
        Mode::Tui if has_terminal => Mode::Tui,
        Mode::Tui => bail!("TUI mode requires an interactive terminal."),
        Mode::Auto if has_gui => Mode::Gui,
        Mode::Auto if has_terminal => Mode::Tui,
        Mode::Auto => bail!("No graphical session or interactive terminal is available."),
    };
    Ok(selected)
}

fn parse_mode() -> anyhow::Result<Mode> {
    parse_mode_args(std::env::args().skip(1))
}

fn parse_mode_args(args: impl IntoIterator<Item = String>) -> anyhow::Result<Mode> {
    let mut mode = Mode::Auto;
    for arg in args {
        let next = match arg.as_str() {
            "--gui" => Mode::Gui,
            "--tui" => Mode::Tui,
            "--help" | "-h" => {
                println!(
                    "filebrowser [--gui | --tui]\n\nAuto-selects the desktop GUI when available, otherwise the terminal UI."
                );
                std::process::exit(0);
            }
            _ => anyhow::bail!("Unknown argument: {arg}"),
        };
        if mode != Mode::Auto && mode != next {
            anyhow::bail!("Use only one of --gui and --tui");
        }
        mode = next;
    }
    Ok(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn auto_mode_prefers_gui_then_terminal() {
        assert_eq!(select_mode(Mode::Auto, true, true).unwrap(), Mode::Gui);
        assert_eq!(select_mode(Mode::Auto, false, true).unwrap(), Mode::Tui);
        assert!(select_mode(Mode::Auto, false, false).is_err());
    }

    #[test]
    fn explicit_modes_require_their_environment() {
        assert_eq!(select_mode(Mode::Gui, true, false).unwrap(), Mode::Gui);
        assert_eq!(select_mode(Mode::Tui, false, true).unwrap(), Mode::Tui);
        assert!(select_mode(Mode::Gui, false, true).is_err());
        assert!(select_mode(Mode::Tui, true, false).is_err());
    }

    #[test]
    fn parses_mode_overrides_and_rejects_conflicting_or_unknown_arguments() {
        assert_eq!(parse_mode_args(args(&["--gui"])).unwrap(), Mode::Gui);
        assert_eq!(parse_mode_args(args(&["--tui"])).unwrap(), Mode::Tui);
        assert!(parse_mode_args(args(&["--gui", "--tui"])).is_err());
        assert!(parse_mode_args(args(&["--unknown"])).is_err());
    }
}
