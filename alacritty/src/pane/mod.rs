//! Independent terminal sessions within a native window.

pub mod geometry;
pub mod layout;

use std::error::Error;
use std::fs::File;
use std::io::Write;
#[cfg(not(windows))]
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use log::info;
use serde_json as json;
use winit::event::Event as WinitEvent;
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use alacritty_terminal::event::Event as TerminalEvent;
use alacritty_terminal::event_loop::{EventLoop as PtyEventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::tty;

use crate::cli::WindowOptions;
use crate::config::UiConfig;
use crate::display::PaneView;
use crate::event::{Event, EventProxy, InlineSearchState, SearchState};

/// Stable identity of a pane within a window. Identities are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub u64);

/// Window mutations are deferred until the input processor releases terminal locks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneCommand {
    Split(layout::Orientation),
    Close,
    Focus(layout::FocusDirection),
}

/// Process a pane's event run until a window mutation is requested; preserve the rest.
pub fn process_batch<E>(
    events: impl IntoIterator<Item = E>,
    mut handle: impl FnMut(E) -> bool,
) -> Vec<E> {
    let mut events = events.into_iter();
    for event in events.by_ref() {
        if handle(event) {
            break;
        }
    }
    events.collect()
}

/// A terminal and its independent PTY session.
pub struct Pane {
    pub view: PaneView,
    pub title: Option<String>,
    pub terminal: Arc<FairMutex<Term<EventProxy>>>,
    pub notifier: Notifier,
    pub cursor_blink_timed_out: bool,
    pub prev_bell_cmd: Option<Instant>,
    pub inline_search_state: InlineSearchState,
    pub search_state: SearchState,
    #[cfg(not(windows))]
    pub master_fd: RawFd,
    #[cfg(not(windows))]
    pub shell_pid: u32,
}

impl Pane {
    pub fn new(
        id: PaneId,
        config: &UiConfig,
        options: &WindowOptions,
        view: PaneView,
        window_id: WindowId,
        proxy: EventLoopProxy<Event>,
    ) -> Result<Self, Box<dyn Error>> {
        let size_info = view.size_info;
        let mut pty_config = config.pty_config();
        options.terminal_options.override_pty_config(&mut pty_config);

        info!("PTY dimensions: {:?} x {:?}", size_info.screen_lines(), size_info.columns());

        let event_proxy = EventProxy::new(proxy, window_id, id);

        // Create the terminal.
        //
        // This object contains all of the state about what's being displayed. It's
        // wrapped in a clonable mutex since both the I/O loop and display need to
        // access it.
        let terminal = Term::new(config.term_options(), &size_info, event_proxy.clone());
        let terminal = Arc::new(FairMutex::new(terminal));

        // Create the PTY.
        //
        // The PTY forks a process to run the shell on the slave side of the
        // pseudoterminal. A file descriptor for the master side is retained for
        // reading/writing to the shell.
        let pty = tty::new(&pty_config, size_info.into(), window_id.into())?;

        #[cfg(not(windows))]
        let master_fd = pty.file().as_raw_fd();
        #[cfg(not(windows))]
        let shell_pid = pty.child().id();

        // Create the pseudoterminal I/O loop.
        //
        // PTY I/O is ran on another thread as to not occupy cycles used by the
        // renderer and input processing. Note that access to the terminal state is
        // synchronized since the I/O loop updates the state, and the display
        // consumes it periodically.
        let event_loop = PtyEventLoop::new(
            Arc::clone(&terminal),
            event_proxy.clone(),
            pty,
            pty_config.drain_on_exit,
            config.debug.ref_test,
        )?;

        // The event loop channel allows write requests from the event processor
        // to be sent to the pty loop and ultimately written to the pty.
        let loop_tx = event_loop.channel();

        // Kick off the I/O thread.
        let _io_thread = event_loop.spawn();

        // Start cursor blinking, in case `Focused` isn't sent on startup.
        if config.cursor.style().blinking {
            event_proxy.send_event(TerminalEvent::CursorBlinkingChange.into());
        }

        Ok(Self {
            view,
            title: None,
            terminal,
            notifier: Notifier(loop_tx),
            cursor_blink_timed_out: Default::default(),
            prev_bell_cmd: Default::default(),
            inline_search_state: Default::default(),
            search_state: Default::default(),
            #[cfg(not(windows))]
            master_fd,
            #[cfg(not(windows))]
            shell_pid,
        })
    }

    /// Write this terminal's grid and dimensions for the existing reference-test harness.
    pub fn write_ref_test_results(&self, directory: &Path) {
        let mut grid = self.terminal.lock().grid().clone();
        grid.initialize_all();
        grid.truncate();
        let serialized_grid = json::to_string(&grid).expect("serialize grid");
        let size = TermSize::new(self.view.size_info.columns(), self.view.size_info.screen_lines());
        let serialized_size = json::to_string(&size).expect("serialize size");
        let serialized_config = format!("{{\"history_size\":{}}}", grid.history_size());
        for (name, contents) in [
            ("grid.json", serialized_grid),
            ("size.json", serialized_size),
            ("config.json", serialized_config),
        ] {
            File::create(directory.join(name))
                .and_then(|mut file| file.write_all(contents.as_bytes()))
                .expect("write pane ref test");
        }
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

/// PTY/timer events keep their originating pane; native input follows focus.
pub fn event_target(event: &WinitEvent<Event>, focused: PaneId) -> PaneId {
    match event {
        WinitEvent::UserEvent(event) => event.pane_id.unwrap_or(focused),
        _ => focused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventType;

    #[test]
    fn terminal_events_keep_their_origin_after_focus_changes() {
        let event =
            Event::new(EventType::BlinkCursor, WindowId::dummy()).with_pane(Some(PaneId(7)));
        let event = WinitEvent::UserEvent(event);
        assert_eq!(event_target(&event, PaneId(2)), PaneId(7));
        assert_eq!(event_target(&event, PaneId(3)), PaneId(7));
    }

    #[test]
    fn unscoped_events_and_native_input_follow_focus() {
        let event = WinitEvent::UserEvent(Event::new(
            EventType::Message(crate::message_bar::Message::new(
                String::new(),
                crate::message_bar::MessageType::Warning,
            )),
            None,
        ));
        assert_eq!(event_target(&event, PaneId(3)), PaneId(3));
        let input = WinitEvent::WindowEvent {
            window_id: WindowId::dummy(),
            event: winit::event::WindowEvent::Focused(true),
        };
        assert_eq!(event_target(&input, PaneId(9)), PaneId(9));
    }
}

#[cfg(test)]
mod batch_tests {
    use super::process_batch;

    #[test]
    fn command_stops_run_without_consuming_following_keyboard_events() {
        let mut handled = Vec::new();
        let remaining = process_batch([1, 2, 3, 4], |event| {
            handled.push(event);
            event == 2
        });
        assert_eq!(handled, [1, 2]);
        assert_eq!(remaining, [3, 4]);
        assert!(process_batch(remaining, |_| false).is_empty());
        assert_eq!(process_batch([1, 2], |_| true), [2]);
    }
}
