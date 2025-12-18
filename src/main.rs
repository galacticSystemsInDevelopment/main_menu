use std::io;
use std::process::{Command, Stdio};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use tui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};

#[derive(Clone)]
enum Screen {
    Main,
    VTMenu,
    Desktops,
    Podman,
    Output,
    Input { prompt: String, origin: PromptOrigin },
}

#[derive(Clone)]
enum PromptOrigin {
    ChangeVT,
    PodmanStart,
    PodmanStop,
    PodmanShell,
}

fn run_capture(cmd: &str, args: &[&str]) -> String {
    match Command::new(cmd).args(args).output() {
        Ok(out) => {
            if out.status.success() {
                String::from_utf8_lossy(&out.stdout).into_owned()
            } else {
                let err = String::from_utf8_lossy(&out.stderr).into_owned();
                if err.trim().is_empty() {
                    format!("Command {:?} {:?} failed: {}", cmd, args, out.status)
                } else {
                    err
                }
            }
        }
        Err(e) => format!("Failed to run {}: {}", cmd, e),
    }
}

fn run_interactive(cmd: &str, args: &[&str]) -> Result<(), String> {
    // restore terminal to allow interactive child to use tty
    disable_raw_mode().map_err(|e| e.to_string())?;
    let status = Command::new(cmd)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| e.to_string());
    // re-enter raw mode for the TUI
    enable_raw_mode().map_err(|e| e.to_string())?;
    status.map(|_| ()).map_err(|e| e)
}

fn chvt(vt: &str) -> String {
    if vt.trim().is_empty() {
        return "empty VT".into();
    }
    // run via PATH lookup (Command::new already searches PATH)
    run_capture("sudo", &["chvt", vt])
}

fn podman_ps() -> String {
    run_capture("podman", &["ps", "-a", "--format", "table {{.ID}}\t{{.Names}}\t{{.Status}}"])
}
fn podman_start(id: &str) -> String {
    run_capture("podman", &["start", id])
}
fn podman_stop(id: &str) -> String {
    run_capture("podman", &["stop", id])
}
fn podman_shell(id: &str) -> Result<(), String> {
    run_interactive("podman", &["exec", "-it", id, "/bin/sh"])
        .or_else(|_| run_interactive("podman", &["exec", "-it", id, "/bin/bash"]))
}

fn draw_menu(
    f: &mut tui::Frame<CrosstermBackend<std::io::Stdout>>,
    title: &str,
    items: &[&str],
    selected: usize,
) {
    let size = f.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [Constraint::Length(3), Constraint::Min(1), Constraint::Length(3)].as_ref(),
        )
        .split(size);

    let header = Paragraph::new(title)
        .block(Block::default().borders(Borders::ALL).title("main_menu"));
    f.render_widget(header, chunks[0]);

    let list: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, &it)| {
            let style = if i == selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(it).style(style)
        })
        .collect();

    let list_widget = List::new(list).block(Block::default().borders(Borders::ALL));
    f.render_widget(list_widget, chunks[1]);

    let footer = Paragraph::new("Arrows: navigate • Enter: select • Esc/q: back/quit")
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}

fn draw_output(
    f: &mut tui::Frame<CrosstermBackend<std::io::Stdout>>,
    title: &str,
    text: &str,
) {
    let size = f.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [Constraint::Length(3), Constraint::Min(1), Constraint::Length(3)].as_ref(),
        )
        .split(size);

    let header = Paragraph::new(title)
        .block(Block::default().borders(Borders::ALL).title("main_menu"));
    f.render_widget(header, chunks[0]);

    let para = Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Output"));
    f.render_widget(para, chunks[1]);

    let footer = Paragraph::new("Esc/Enter to go back").block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}

fn main() -> Result<(), io::Error> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut screen = Screen::Main;
    let mut selected = 0usize;
    let mut message = String::new();
    let mut input = String::new();
    let mut input_origin = PromptOrigin::ChangeVT;

    loop {
        terminal.draw(|f| {
            match &screen {
                Screen::Main => {
                    draw_menu(f, "Main Menu", &["VT Menu", "Podman Menu", "Quit"], selected)
                }
                Screen::VTMenu => draw_menu(
                    f,
                    "VT Menu",
                    &["Change VT (ask number)", "Desktops", "Back to Main Menu"],
                    selected,
                ),
                Screen::Desktops => draw_menu(
                    f,
                    "Desktops",
                    &[
                        "Known: X11 VT (chvt 7)",
                        "Known: Wayland VT (chvt 8)",
                        "Back to VT Menu",
                    ],
                    selected,
                ),
                Screen::Podman => draw_menu(
                    f,
                    "Podman Menu",
                    &[
                        "List containers",
                        "Start container",
                        "Stop container",
                        "Open shell in container",
                        "Back to Main Menu",
                    ],
                    selected,
                ),
                Screen::Output => draw_output(f, "Output", &message),
                Screen::Input { prompt, .. } => {
                    let size = f.size();
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .margin(2)
                        .constraints(
                            [Constraint::Length(3), Constraint::Min(1), Constraint::Length(3)]
                                .as_ref(),
                        )
                        .split(size);
                    let header = Paragraph::new(prompt.as_str())
                        .block(Block::default().borders(Borders::ALL).title("main_menu"));
                    f.render_widget(header, chunks[0]);
                    let para = Paragraph::new(input.as_str())
                        .block(Block::default().borders(Borders::ALL).title("Input"));
                    f.render_widget(para, chunks[1]);
                    let footer = Paragraph::new("Type and press Enter. Esc to cancel.")
                        .block(Block::default().borders(Borders::ALL));
                    f.render_widget(footer, chunks[2]);
                }
            }
        })?;

        if event::poll(std::time::Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => {
                    match &mut screen {
                        Screen::Main => match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => break,
                            KeyCode::Down => selected = (selected + 1) % 3,
                            KeyCode::Up => selected = (selected + 3 - 1) % 3,
                            KeyCode::Enter => match selected {
                                0 => {
                                    screen = Screen::VTMenu;
                                    selected = 0;
                                }
                                1 => {
                                    screen = Screen::Podman;
                                    selected = 0;
                                }
                                2 => break,
                                _ => {}
                            },
                            _ => {}
                        },
                        Screen::VTMenu => match key.code {
                            KeyCode::Esc => {
                                screen = Screen::Main;
                                selected = 0;
                            }
                            KeyCode::Down => selected = (selected + 1) % 3,
                            KeyCode::Up => selected = (selected + 3 - 1) % 3,
                            KeyCode::Enter => match selected {
                                0 => {
                                    input.clear();
                                    input_origin = PromptOrigin::ChangeVT;
                                    screen = Screen::Input {
                                        prompt: "Enter VT number (e.g. 1..12)".into(),
                                        origin: input_origin.clone(),
                                    };
                                }
                                1 => {
                                    screen = Screen::Desktops;
                                    selected = 0;
                                }
                                2 => {
                                    screen = Screen::Main;
                                    selected = 0;
                                }
                                _ => {}
                            },
                            _ => {}
                        },
                        Screen::Desktops => match key.code {
                            KeyCode::Esc => {
                                screen = Screen::VTMenu;
                                selected = 0;
                            }
                            KeyCode::Down => selected = (selected + 1) % 3,
                            KeyCode::Up => selected = (selected + 3 - 1) % 3,
                            KeyCode::Enter => match selected {
                                0 => {
                                    message = chvt("7");
                                    screen = Screen::Output;
                                }
                                1 => {
                                    message = chvt("8");
                                    screen = Screen::Output;
                                }
                                2 => {
                                    screen = Screen::VTMenu;
                                    selected = 0;
                                }
                                _ => {}
                            },
                            _ => {}
                        },
                        Screen::Podman => match key.code {
                            KeyCode::Esc => {
                                screen = Screen::Main;
                                selected = 0;
                            }
                            KeyCode::Down => selected = (selected + 1) % 5,
                            KeyCode::Up => selected = (selected + 5 - 1) % 5,
                            KeyCode::Enter => match selected {
                                0 => {
                                    message = podman_ps();
                                    screen = Screen::Output;
                                }
                                1 => {
                                    input.clear();
                                    input_origin = PromptOrigin::PodmanStart;
                                    screen = Screen::Input {
                                        prompt: "Start container (id or name)".into(),
                                        origin: input_origin.clone(),
                                    };
                                }
                                2 => {
                                    input.clear();
                                    input_origin = PromptOrigin::PodmanStop;
                                    screen = Screen::Input {
                                        prompt: "Stop container (id or name)".into(),
                                        origin: input_origin.clone(),
                                    };
                                }
                                3 => {
                                    input.clear();
                                    input_origin = PromptOrigin::PodmanShell;
                                    screen = Screen::Input {
                                        prompt: "Open shell in container (id or name)".into(),
                                        origin: input_origin.clone(),
                                    };
                                }
                                4 => {
                                    screen = Screen::Main;
                                    selected = 0;
                                }
                                _ => {}
                            },
                            _ => {}
                        },
                        Screen::Output => match key.code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                                // return to Podman by default (keeps original behavior)
                                screen = Screen::Podman;
                                selected = 0;
                                message.clear();
                            }
                            _ => {}
                        },
                        Screen::Input { origin, .. } => {
                            // clone the origin to avoid borrowing `screen` while reassigning it
                            let origin_clone = origin.clone();
                            match key.code {
                                KeyCode::Esc => {
                                    match origin_clone {
                                        PromptOrigin::ChangeVT => {
                                            screen = Screen::VTMenu;
                                            selected = 0;
                                        }
                                        _ => {
                                            screen = Screen::Podman;
                                            selected = 0;
                                        }
                                    }
                                }
                                KeyCode::Enter => {
                                    let val = input.trim().to_string();
                                    if val.is_empty() {
                                        message = "Empty input".into();
                                        screen = Screen::Output;
                                    } else {
                                        match origin_clone {
                                            PromptOrigin::ChangeVT => {
                                                message = chvt(&val);
                                                screen = Screen::Output;
                                            }
                                            PromptOrigin::PodmanStart => {
                                                message = podman_start(&val);
                                                screen = Screen::Output;
                                            }
                                            PromptOrigin::PodmanStop => {
                                                message = podman_stop(&val);
                                                screen = Screen::Output;
                                            }
                                            PromptOrigin::PodmanShell => {
                                                match podman_shell(&val) {
                                                    Ok(()) => message = format!("Exited shell for {}", val),
                                                    Err(e) => message = e,
                                                }
                                                screen = Screen::Output;
                                            }
                                        }
                                    }
                                    input.clear();
                                }
                                KeyCode::Backspace => {
                                    input.pop();
                                }
                                KeyCode::Char(c) => {
                                    input.push(c);
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Event::Mouse(_) => {}
                Event::Resize(_, _) => {}
                Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
                _ => {}
            }
        }
    }

    // restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    Ok(())
}
