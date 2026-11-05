//! Terminal window context.

use std::collections::BTreeMap;
use std::error::Error;
use std::mem;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use glutin::config::Config as GlutinConfig;
use glutin::display::GetGlDisplay;
#[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
use glutin::platform::x11::X11GlConfigExt;
use winit::dpi::PhysicalPosition;
use winit::event::{Event as WinitEvent, Modifiers, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::WindowId;

use alacritty_terminal::event::Event as TerminalEvent;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Direction;
use alacritty_terminal::term::{Term, TermMode};

use crate::cli::{ParsedOptions, WindowOptions};
use crate::clipboard::Clipboard;
use crate::config::UiConfig;
use crate::display::window::Window;
use crate::display::{Display, PaneView};
use crate::event::{ActionContext, Event, EventProxy, Mouse, TouchPurpose};
#[cfg(unix)]
use crate::logging::LOG_TARGET_IPC_CONFIG;
use crate::message_bar::MessageBuffer;
use crate::pane::geometry::hit_test;
use crate::pane::{Pane, PaneId, event_target};
use crate::scheduler::Scheduler;
use crate::{input, renderer};

/// Event context for one individual Alacritty window.
pub struct WindowContext {
    pub message_buffer: MessageBuffer,
    pub display: Display,
    pub dirty: bool,
    event_queue: Vec<WinitEvent<Event>>,
    panes: BTreeMap<PaneId, Pane>,
    focused: PaneId,
    next_pane_id: u64,
    modifiers: Modifiers,
    mouse: Mouse,
    pointer_pane: Option<PaneId>,
    touch: TouchPurpose,
    occluded: bool,
    preserve_title: bool,
    window_config: ParsedOptions,
    config: Rc<UiConfig>,
}

impl WindowContext {
    /// The active terminal session for this window.
    fn focused_pane(&self) -> &Pane {
        self.panes.get(&self.focused).expect("focused pane")
    }

    /// Check the origin of asynchronous events before dispatching them.
    pub fn has_pane(&self, id: PaneId) -> bool {
        self.panes.contains_key(&id)
    }

    /// Returns true when the last pane exited and the window should close.
    pub fn pane_exited(&mut self, pane_id: Option<PaneId>, scheduler: &mut Scheduler) -> bool {
        let pane_id = pane_id.unwrap_or(self.focused);
        if !self.has_pane(pane_id) {
            return false;
        }
        if self.panes.len() == 1 {
            // Retain the terminal for --hold and reference-test serialization.
            return !self.display.window.hold;
        }
        self.panes.remove(&pane_id);
        if self.pointer_pane == Some(pane_id) {
            self.pointer_pane = None;
        }
        scheduler.unschedule_pane(self.id(), pane_id);
        self.event_queue.retain(|event| {
            !matches!(event, WinitEvent::UserEvent(event) if event.pane_id == Some(pane_id))
        });
        if self.focused == pane_id {
            // Pointer/gesture state belongs to the previously focused terminal.
            self.mouse = Mouse::default();
            self.touch = TouchPurpose::default();
            self.focused = *self.panes.first_key_value().expect("surviving pane").0;
        }
        self.dirty = true;
        self.display.window.request_redraw();
        false
    }

    /// Create initial window context that does bootstrapping the graphics API we're going to use.
    pub fn initial(
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<Event>,
        config: Rc<UiConfig>,
        mut options: WindowOptions,
    ) -> Result<Self, Box<dyn Error>> {
        let raw_display_handle = event_loop.display_handle().unwrap().as_raw();

        let mut identity = config.window.identity.clone();
        options.window_identity.override_identity_config(&mut identity);

        // Windows has different order of GL platform initialization compared to any other platform;
        // it requires the window first.
        #[cfg(windows)]
        let window = Window::new(event_loop, &config, &identity, &mut options)?;
        #[cfg(windows)]
        let raw_window_handle = Some(window.raw_window_handle());

        #[cfg(not(windows))]
        let raw_window_handle = None;

        let gl_display = renderer::platform::create_gl_display(
            raw_display_handle,
            raw_window_handle,
            config.debug.prefer_egl,
        )?;
        let gl_config = renderer::platform::pick_gl_config(&gl_display, raw_window_handle)?;

        #[cfg(not(windows))]
        let window = Window::new(
            event_loop,
            &config,
            &identity,
            &mut options,
            #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
            gl_config.x11_visual(),
        )?;

        // Create context.
        let gl_context =
            renderer::platform::create_gl_context(&gl_display, &gl_config, raw_window_handle)?;

        let (display, pane_view) = Display::new(window, gl_context, &config, false)?;

        Self::new(display, pane_view, config, options, proxy)
    }

    /// Create additional context with the graphics platform other windows are using.
    pub fn additional(
        gl_config: &GlutinConfig,
        event_loop: &ActiveEventLoop,
        proxy: EventLoopProxy<Event>,
        config: Rc<UiConfig>,
        mut options: WindowOptions,
        config_overrides: ParsedOptions,
    ) -> Result<Self, Box<dyn Error>> {
        let gl_display = gl_config.display();

        let mut identity = config.window.identity.clone();
        options.window_identity.override_identity_config(&mut identity);

        // Check if new window will be opened as a tab.
        // This must be done before `Window::new()`, which unsets `window_tabbing_id`.
        #[cfg(target_os = "macos")]
        let tabbed = options.window_tabbing_id.is_some();
        #[cfg(not(target_os = "macos"))]
        let tabbed = false;

        let window = Window::new(
            event_loop,
            &config,
            &identity,
            &mut options,
            #[cfg(all(feature = "x11", not(any(target_os = "macos", windows))))]
            gl_config.x11_visual(),
        )?;

        // Create context.
        let raw_window_handle = window.raw_window_handle();
        let gl_context =
            renderer::platform::create_gl_context(&gl_display, gl_config, Some(raw_window_handle))?;

        let (display, pane_view) = Display::new(window, gl_context, &config, tabbed)?;

        let mut window_context = Self::new(display, pane_view, config, options, proxy)?;

        // Set the config overrides at startup.
        //
        // These are already applied to `config`, so no update is necessary.
        window_context.window_config = config_overrides;

        Ok(window_context)
    }

    /// Create a new terminal window context.
    fn new(
        display: Display,
        pane_view: PaneView,
        config: Rc<UiConfig>,
        options: WindowOptions,
        proxy: EventLoopProxy<Event>,
    ) -> Result<Self, Box<dyn Error>> {
        let preserve_title = options.window_identity.title.is_some();
        let focused = PaneId(0);
        let mut context = WindowContext {
            preserve_title,
            panes: Default::default(),
            focused,
            next_pane_id: 0,
            display,
            config,
            message_buffer: Default::default(),
            window_config: Default::default(),
            event_queue: Default::default(),
            modifiers: Default::default(),
            occluded: Default::default(),
            mouse: Default::default(),
            pointer_pane: None,
            touch: Default::default(),
            dirty: Default::default(),
        };
        context.focused = context.allocate_pane_id();
        let pane = Pane::new(
            context.focused,
            &context.config,
            &options,
            pane_view,
            context.display.window.id(),
            proxy,
        )?;
        context.panes.insert(context.focused, pane);
        Ok(context)
    }

    /// Reserve a unique identity, including for failed future pane creations.
    fn allocate_pane_id(&mut self) -> PaneId {
        let id = PaneId(self.next_pane_id);
        self.next_pane_id = self.next_pane_id.checked_add(1).expect("pane ID space exhausted");
        id
    }

    /// Update the terminal window to the latest config.
    pub fn update_config(&mut self, new_config: Rc<UiConfig>) {
        let old_config = mem::replace(&mut self.config, new_config);

        // Apply ipc config if there are overrides.
        self.config = self.window_config.override_config_rc(self.config.clone());

        for pane in self.panes.values_mut() {
            pane.view.update_config(&self.config);
            pane.terminal.lock().set_options(self.config.term_options());
            pane.view.hint_state.update_alphabet(self.config.hints.alphabet());
        }

        // Reload cursor if its thickness has changed.
        if (old_config.cursor.thickness() - self.config.cursor.thickness()).abs() > f32::EPSILON {
            self.display.pending_update.set_cursor_dirty();
        }

        if old_config.font != self.config.font {
            let scale_factor = self.display.window.scale_factor as f32;
            // Do not update font size if it has been changed at runtime.
            if self.display.font_size == old_config.font.size().scale(scale_factor) {
                self.display.font_size = self.config.font.size().scale(scale_factor);
            }

            let font = self.config.font.clone().with_size(self.display.font_size);
            self.display.pending_update.set_font(font);
        }

        // Always reload the theme to account for auto-theme switching.
        self.display.window.set_theme(self.config.window.theme());

        // Update display if either padding options or resize increments were changed.
        let window_config = &old_config.window;
        if window_config.padding(1.) != self.config.window.padding(1.)
            || window_config.dynamic_padding != self.config.window.dynamic_padding
            || window_config.resize_increments != self.config.window.resize_increments
        {
            self.display.pending_update.dirty = true;
        }

        // Update title on config reload according to the following table.
        //
        // │cli │ dynamic_title │ current_title == old_config ││ set_title │
        // │ Y  │       _       │              _              ││     N     │
        // │ N  │       Y       │              Y              ││     Y     │
        // │ N  │       Y       │              N              ││     N     │
        // │ N  │       N       │              _              ││     Y     │
        if !self.preserve_title
            && (!self.config.window.dynamic_title
                || self.display.window.title() == old_config.window.identity.title)
        {
            self.display.window.set_title(self.config.window.identity.title.clone());
        }

        let opaque = self.config.window_opacity() >= 1.;

        // Disable shadows for transparent windows on macOS.
        #[cfg(target_os = "macos")]
        self.display.window.set_has_shadow(opaque);

        #[cfg(target_os = "macos")]
        self.display.window.set_option_as_alt(self.config.window.option_as_alt());

        // Change opacity and blur state.
        self.display.window.set_transparent(!opaque);
        self.display.window.set_blur(self.config.window.blur);

        // Blink events carry the same identity as PTY-originated events.
        for &id in self.panes.keys() {
            let event = Event::new(TerminalEvent::CursorBlinkingChange.into(), self.id())
                .with_pane(Some(id));
            self.event_queue.push(event.into());
        }

        self.dirty = true;
    }

    /// Get reference to the window's configuration.
    #[cfg(unix)]
    pub fn config(&self) -> &UiConfig {
        &self.config
    }

    /// Clear the window config overrides.
    #[cfg(unix)]
    pub fn reset_window_config(&mut self, config: Rc<UiConfig>) {
        // Clear previous window errors.
        self.message_buffer.remove_target(LOG_TARGET_IPC_CONFIG);

        self.window_config.clear();

        // Reload current config to pull new IPC config.
        self.update_config(config);
    }

    /// Add new window config overrides.
    #[cfg(unix)]
    pub fn add_window_config(&mut self, config: Rc<UiConfig>, options: &ParsedOptions) {
        // Clear previous window errors.
        self.message_buffer.remove_target(LOG_TARGET_IPC_CONFIG);

        self.window_config.extend_from_slice(options);

        // Reload current config to pull new IPC config.
        self.update_config(config);
    }

    /// Draw the window.
    pub fn draw(&mut self, scheduler: &mut Scheduler) {
        self.display.window.requested_redraw = false;

        if self.occluded {
            return;
        }

        self.dirty = false;

        // Force the display to process any pending display update.
        let pane = self.panes.get_mut(&self.focused).expect("focused pane");
        self.display.process_renderer_update(&mut pane.view);

        // Request immediate re-draw if visual bell animation is not finished yet.
        if !pane.view.visual_bell.completed() {
            // We can get an OS redraw which bypasses alacritty's frame throttling, thus
            // marking the window as dirty when we don't have frame yet.
            if self.display.window.has_frame {
                self.display.window.request_redraw();
            } else {
                self.dirty = true;
            }
        }

        // Clear the native surface before the pane's scissored clear/draw.
        if self.display.begin_frame(&self.config) {
            pane.view.damage_tracker.frame().mark_fully_damaged();
        }

        // Redraw the window.
        let terminal = pane.terminal.lock();
        self.display.draw(
            &mut pane.view,
            terminal,
            scheduler,
            &self.message_buffer,
            &self.config,
            &mut pane.search_state,
        );
    }

    /// Hit-test in window coordinates before giving the input processor local pixels.
    fn translate_pointer_event(&mut self, event: &mut WinitEvent<Event>) {
        let position = match event {
            WinitEvent::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. }, ..
            } => position,
            WinitEvent::WindowEvent { event: WindowEvent::Touch(touch), .. } => &mut touch.location,
            _ => return,
        };
        let window_position = (position.x, position.y);
        self.pointer_pane =
            hit_test(self.panes.iter().map(|(&id, pane)| (id, pane.view.rect)), window_position);
        // Input remains focused-pane scoped; click-to-focus policy comes with split panes.
        // Keeping out-of-bounds local coordinates preserves selection dragging.
        let local = self.focused_pane().view.rect.local_position(window_position);
        *position = PhysicalPosition::new(local.0, local.1);
    }

    /// Process events for this terminal window.
    pub fn handle_event(
        &mut self,
        #[cfg(target_os = "macos")] event_loop: &ActiveEventLoop,
        event_proxy: &EventLoopProxy<Event>,
        clipboard: &mut Clipboard,
        scheduler: &mut Scheduler,
        event: WinitEvent<Event>,
    ) {
        match event {
            WinitEvent::AboutToWait
            | WinitEvent::WindowEvent { event: WindowEvent::RedrawRequested, .. } => {
                // Skip further event handling with no staged updates.
                if self.event_queue.is_empty() {
                    return;
                }

                // Continue to process all pending events.
            },
            mut event => {
                self.translate_pointer_event(&mut event);
                self.event_queue.push(event);
                return;
            },
        }

        while let Some(next) = self.event_queue.first() {
            let pane_id = event_target(next, self.focused);
            let count = self
                .event_queue
                .iter()
                .take_while(|event| event_target(event, self.focused) == pane_id)
                .count();
            // Borrow fields separately so pane state and the shared display can be mutated
            // together.
            let pane = match self.panes.get_mut(&pane_id) {
                Some(pane) => pane,
                None => {
                    self.event_queue.drain(..count);
                    continue;
                },
            };
            let terminal = Arc::clone(&pane.terminal);
            let mut terminal = terminal.lock();

            let old_is_searching = pane.search_state.history_index.is_some();

            let context = ActionContext {
                pane_id,
                cursor_blink_timed_out: &mut pane.cursor_blink_timed_out,
                prev_bell_cmd: &mut pane.prev_bell_cmd,
                message_buffer: &mut self.message_buffer,
                inline_search_state: &mut pane.inline_search_state,
                search_state: &mut pane.search_state,
                modifiers: &mut self.modifiers,
                notifier: &mut pane.notifier,
                display: &mut self.display,
                pane_view: &mut pane.view,
                mouse: &mut self.mouse,
                touch: &mut self.touch,
                dirty: &mut self.dirty,
                occluded: &mut self.occluded,
                terminal: &mut terminal,
                #[cfg(not(windows))]
                master_fd: pane.master_fd,
                #[cfg(not(windows))]
                shell_pid: pane.shell_pid,
                preserve_title: self.preserve_title,
                config: &self.config,
                event_proxy,
                #[cfg(target_os = "macos")]
                event_loop,
                clipboard,
                scheduler,
            };
            let mut processor = input::Processor::new(context);

            for event in self.event_queue.drain(..count) {
                processor.handle_event(event);
            }

            // Shared font/window changes must reach every pane, not only this batch's target.
            if self.display.pending_update.dirty {
                self.submit_display_update(pane_id, old_is_searching, &mut terminal);
                self.dirty = true;
            }

            let pane = self.panes.get_mut(&pane_id).expect("event pane");
            if self.dirty || self.mouse.hint_highlight_dirty {
                self.dirty |= self.display.update_highlighted_hints(
                    &mut pane.view,
                    &terminal,
                    &self.config,
                    &self.mouse,
                    self.modifiers.state(),
                );
                self.mouse.hint_highlight_dirty = false;
            }
        }

        // Don't call `request_redraw` when event is `RedrawRequested` since the `dirty` flag
        // represents the current frame, but redraw is for the next frame.
        if self.dirty
            && self.display.window.has_frame
            && !self.occluded
            && !matches!(event, WinitEvent::WindowEvent { event: WindowEvent::RedrawRequested, .. })
        {
            self.display.window.request_redraw();
        }
    }

    /// ID of this terminal context.
    pub fn id(&self) -> WindowId {
        self.display.window.id()
    }

    /// Preserve original single-pane output paths; give additional panes distinct directories.
    pub fn write_ref_test_results(&self) {
        for (&id, pane) in &self.panes {
            if self.panes.len() == 1 {
                pane.write_ref_test_results(Path::new("."));
            } else {
                let directory = format!("pane-{}", id.0);
                std::fs::create_dir_all(&directory).expect("create pane ref-test directory");
                pane.write_ref_test_results(Path::new(&directory));
            }
        }
    }

    /// Apply a window update once, then resize every terminal using its own state.
    fn submit_display_update(
        &mut self,
        event_pane: PaneId,
        old_is_searching: bool,
        event_terminal: &mut Term<EventProxy>,
    ) {
        let invalidate = self.display.handle_update(&self.config);
        let single_pane = self.panes.len() == 1;
        for (&id, pane) in &mut self.panes {
            // Phase 0 has one full-window pane. The layout tree will supply bounds in Phase 2.
            if single_pane {
                pane.view.rect.width = self.display.window_size.width;
                pane.view.rect.height = self.display.window_size.height;
            }
            if invalidate {
                pane.view.damage_tracker.frame().mark_fully_damaged();
            }
            // Keep the input batch's guard held through resize and hint refresh. Other PTYs
            // can be locked independently; no PTY thread ever locks another terminal.
            let background = Arc::clone(&pane.terminal);
            let mut background_terminal;
            let terminal = if id == event_pane {
                &mut *event_terminal
            } else {
                background_terminal = background.lock();
                &mut *background_terminal
            };
            let was_searching = if id == event_pane {
                old_is_searching
            } else {
                pane.search_state.history_index.is_some()
            };
            let num_lines = terminal.screen_lines();
            let cursor_at_bottom = terminal.grid().cursor.point.line + 1 == num_lines;
            let origin_at_bottom = if terminal.mode().contains(TermMode::VI) {
                terminal.vi_mode_cursor.point.line == num_lines - 1
            } else {
                pane.search_state.direction == Direction::Left
            };

            self.display.resize_pane(
                &mut pane.view,
                terminal,
                &mut pane.notifier,
                &self.message_buffer,
                &mut pane.search_state,
                &self.config,
            );

            if !was_searching && pane.search_state.history_index.is_some() {
                let display_offset = terminal.grid().display_offset();
                if display_offset == 0 && cursor_at_bottom && !origin_at_bottom {
                    terminal.scroll_display(Scroll::Delta(1));
                } else if display_offset != 0 && origin_at_bottom {
                    terminal.scroll_display(Scroll::Delta(-1));
                }
            }
        }
    }
}
