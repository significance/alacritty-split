//! Independent terminal sessions within a native window.

use std::error::Error;
#[cfg(not(windows))]
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::Arc;
use std::time::Instant;

use log::info;
use winit::event_loop::EventLoopProxy;
use winit::window::WindowId;

use alacritty_terminal::event::Event as TerminalEvent;
use alacritty_terminal::event_loop::{EventLoop as PtyEventLoop, Msg, Notifier};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::tty;

use crate::cli::WindowOptions;
use crate::config::UiConfig;
use crate::display::SizeInfo;
use crate::event::{Event, EventProxy, InlineSearchState, SearchState};

/// Stable identity of a pane within a window. Identities are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PaneId(pub u64);

/// A terminal and its independent PTY session.
pub struct Pane {
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
        config: &UiConfig,
        options: &WindowOptions,
        size_info: SizeInfo,
        window_id: WindowId,
        proxy: EventLoopProxy<Event>,
    ) -> Result<Self, Box<dyn Error>> {
        let mut pty_config = config.pty_config();
        options.terminal_options.override_pty_config(&mut pty_config);

        info!("PTY dimensions: {:?} x {:?}", size_info.screen_lines(), size_info.columns());

        let event_proxy = EventProxy::new(proxy, window_id);

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
}

impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}
