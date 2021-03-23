// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

#[allow(dead_code)]
mod util;

use crate::util::{
    event::{Event, Events},
    TabsState,
};
use std::{error::Error, io};
use termion::{event::Key, input::MouseTerminal, raw::IntoRawMode, screen::AlternateScreen};
use tui::{
    backend::Backend,
    backend::TermionBackend,
    layout::Rect,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Tabs},
    Frame, Terminal,
};

type F = tui::backend::TermionBackend<
    termion::screen::AlternateScreen<
        termion::input::MouseTerminal<termion::raw::RawTerminal<std::io::Stdout>>,
    >,
>;
trait Window {
    fn title(&self) -> &str;
    fn draw(&self, b: &mut Frame<F>, area: Rect);
    fn handle_input(&self, i: Key) -> bool;
    fn activate(&mut self);
}

struct DefaultWindow {
    activated: bool,
}

impl Window for DefaultWindow {
    fn title(&self) -> &str {
        if !self.activated {
            "Default Window"
        } else {
            "Active Window"
        }
    }
    fn draw(&self, b: &mut Frame<F>, area: Rect) {
        psbt(b, area, 6);
    }
    fn handle_input(&self, i: Key) -> bool {
        false
    }
    fn activate(&mut self) {
        self.activated = true;
    }
}

pub struct WindowList {
    pub windows: Vec<Box<dyn Window>>,
    index: usize,
}

impl WindowList {
    pub fn new(windows: Vec<Box<dyn Window>>) -> WindowList {
        WindowList { windows, index: 0 }
    }
    pub fn next(&mut self) {
        self.index = (self.index + 1) % self.windows.len();
    }

    pub fn previous(&mut self) {
        self.index = (self.index + self.windows.len() - 1) % self.windows.len();
    }
    fn draw(&self, b: &mut Frame<F>, area: Rect) {
        self.windows[self.index].draw(b, area)
    }
    fn handle_input(&self, i: Key) -> bool {
        self.windows[self.index].handle_input(i)
    }
    fn activate(&mut self) {
        self.windows[self.index].activate()
    }
}

struct App {
    tabs: WindowList,
}

fn main() -> Result<(), Box<dyn Error>> {
    // Terminal initialization
    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut events = Events::new();

    // App
    let mut app = App {
        tabs: WindowList::new(vec![
            Box::new(DefaultWindow { activated: false }),
            Box::new(DefaultWindow { activated: false }),
        ]),
    };

    let mut hijack_input = false;
    // Main loop
    loop {
        terminal.draw(|f| {
            let size = f.size();
            let block = Block::default().style(Style::default().bg(Color::White).fg(Color::Black));
            f.render_widget(block, size);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(5)
                .constraints([Constraint::Length(10), Constraint::Min(0)].as_ref())
                .split(size);
            let titles = app
                .tabs
                .windows
                .iter()
                .map(|window| {
                    let t = window.title();
                    let (first, rest) = t.split_at(1);
                    Spans::from(vec![
                        Span::styled(first, Style::default().fg(Color::Yellow)),
                        Span::styled(rest, Style::default().fg(Color::Green)),
                    ])
                })
                .collect();
            let tabs = Tabs::new(titles)
                .block(Block::default().borders(Borders::ALL).title("Tabs"))
                .select(app.tabs.index)
                .style(Style::default().fg(Color::Cyan))
                .highlight_style(
                    Style::default()
                        .add_modifier(Modifier::BOLD)
                        .bg(Color::Black),
                );
            f.render_widget(tabs, chunks[0]);
            app.tabs.draw(f, chunks[1]);
        })?;

        if let Event::Input(input) = events.next()? {
            if !hijack_input {
                match input {
                    Key::Esc => {
                        break;
                    }
                    Key::Right => app.tabs.next(),
                    Key::Left => app.tabs.previous(),
                    Key::Down => {
                        hijack_input = true;
                        events.disable_exit_key();
                        app.tabs.activate();
                    }
                    _ => {}
                }
            } else {
                hijack_input = app.tabs.handle_input(input);
            }
        }
    }
    Ok(())
}

fn psbt<B: Backend>(f: &mut Frame<B>, r: Rect, n: u8) -> () {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(5)
        .constraints(vec![Constraint::Length(5); n as usize].as_ref())
        .split(r);

    let tabs = Tabs::new(vec![Spans::from(vec![Span::raw("A")])])
        .block(Block::default().borders(Borders::ALL).title("Tabs"))
        .select(1)
        .style(Style::default().fg(Color::Cyan))
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .bg(Color::Black),
        );
    for i in 0..n {
        f.render_widget(tabs.clone(), chunks[i as usize]);
    }
}
