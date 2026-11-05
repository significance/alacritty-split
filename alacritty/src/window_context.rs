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
use log::{error, warn};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Event as WinitEvent, Modifiers, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::raw_window_handle::HasDisplayHandle;
use winit::window::{CursorIcon, WindowId};

use alacritty_terminal::event::{Event as TerminalEvent, Notify};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Direction;
use alacritty_terminal::term::{Term, TermMode};

use crate::cli::{ParsedOptions, WindowOptions};
use crate::clipboard::Clipboard;
use crate::config::UiConfig;
use crate::display::window::{ImeInhibitor, Window};
use crate::display::{Display, PaneView};
use crate::event::{ActionContext, Event, EventProxy, Mouse, TouchPurpose};
#[cfg(unix)]
use crate::logging::LOG_TARGET_IPC_CONFIG;
use crate::message_bar::MessageBuffer;
use crate::pane::geometry::{PaneRect, hit_test};
use crate::pane::layout::{Layout, LayoutGeometry, Orientation};
use crate::pane::pointer::{ButtonAction, PointerState};
use crate::pane::{Pane, PaneCommand, PaneId, event_target};
use crate::scheduler::{Scheduler, TimerId, Topic};
use crate::{input, renderer};

/// Event context for one individual Alacritty window.
pub struct WindowContext {
    pub message_buffer: MessageBuffer,
    pub display: Display,
    pub dirty: bool,
    event_queue: Vec<WinitEvent<Event>>,
    panes: BTreeMap<PaneId, Pane>,
    focused: PaneId,
    layout: Layout,
    geometry: LayoutGeometry,
    next_pane_id: u64,
    modifiers: Modifiers,
    mouse: Mouse,
    pointer_pane: Option<PaneId>,
    pointer: PointerState,
    seed_pointer: bool,
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
        let params = self.valid_layout_params();
        let next_focus = match self.layout.remove(pane_id, params) {
            Ok(id) => id,
            Err(error) => {
                error!("Cannot remove exited pane with geometry: {error:?}; recovering tree");
                match self.layout.discard(pane_id) {
                    Ok(id) => id,
                    Err(error) => {
                        // Recover even a corrupt tree using every surviving terminal.
                        error!("Cannot discard exited pane: {error:?}; rebuilding layout");
                        let survivors: Vec<_> =
                            self.panes.keys().copied().filter(|id| *id != pane_id).collect();
                        self.layout = Layout::from_panes(&survivors);
                        survivors[0]
                    },
                }
            },
        };
        if self.focused == pane_id {
            self.focus_pane(next_focus, scheduler);
        }
        self.pointer.drag = None;
        self.panes.remove(&pane_id);
        if self.pointer_pane == Some(pane_id) {
            self.pointer_pane = None;
        }
        scheduler.unschedule_pane(self.id(), pane_id);
        self.event_queue.retain(|event| {
            !matches!(event, WinitEvent::UserEvent(event) if event.pane_id == Some(pane_id))
        });
        self.refresh_pane_sizes();
        self.dirty = true;
        self.display.window.request_redraw();
        false
    }

    fn valid_layout_params(&self) -> crate::pane::layout::LayoutParams {
        let mut params = self.display.layout_params(&self.config, &self.message_buffer);
        if let Ok(minimum) = self.layout.minimum(params) {
            params.bounds.width = params.bounds.width.max(minimum.width);
            params.bounds.height = params.bounds.height.max(minimum.height);
        }
        params
    }

    fn refresh_pane_sizes(&mut self) {
        let terminal = Arc::clone(&self.focused_pane().terminal);
        let mut terminal = terminal.lock();
        let searching = self.focused_pane().search_state.history_index.is_some();
        self.submit_display_update(self.focused, searching, &mut terminal);
        self.dirty = true;
    }

    fn focus_pane(&mut self, id: PaneId, scheduler: &mut Scheduler) {
        if id == self.focused || !self.panes.contains_key(&id) {
            return;
        }
        let previous = self.focused;
        self.pointer.cancel_gesture();
        scheduler.unschedule(
            TimerId::new(Topic::SelectionScrolling, self.id()).with_pane(Some(previous)),
        );

        let old = self.panes.get_mut(&previous).expect("focused pane");
        let native_focused = {
            let mut terminal = old.terminal.lock();
            let focused = terminal.is_focused;
            if focused && terminal.mode().contains(TermMode::FOCUS_IN_OUT) {
                old.notifier.notify(b"\x1b[O".to_vec());
            }
            terminal.is_focused = false;
            focused
        };
        old.view.damage_tracker.frame().mark_fully_damaged();
        // Composition belongs to the native window and follows its active terminal.
        let ime = mem::take(&mut old.view.ime);
        let new = self.panes.get_mut(&id).expect("focus target");
        new.view.ime = ime;
        {
            let mut terminal = new.terminal.lock();
            terminal.is_focused = native_focused;
            let vi_inhibited = terminal.mode().contains(TermMode::VI)
                && new.search_state.history_index.is_none()
                && !new.inline_search_state.char_pending;
            self.display.window.set_ime_inhibitor(ImeInhibitor::VI, vi_inhibited);
            if native_focused && terminal.mode().contains(TermMode::FOCUS_IN_OUT) {
                new.notifier.notify(b"\x1b[I".to_vec());
            }
        }
        new.view.damage_tracker.frame().mark_fully_damaged();
        new.cursor_blink_timed_out = false;
        new.view.cursor_hidden = false;
        if !self.preserve_title && self.config.window.dynamic_title {
            let title =
                new.title.clone().unwrap_or_else(|| self.config.window.identity.title.clone());
            self.display.window.set_title(title);
        }
        self.focused = id;
        self.mouse = Mouse::default();
        self.touch = TouchPurpose::default();
        for pane_id in [previous, id] {
            self.event_queue.push(
                Event::new(TerminalEvent::CursorBlinkingChange.into(), self.id())
                    .with_pane(Some(pane_id))
                    .into(),
            );
        }
        self.dirty = true;
    }

    fn apply_pane_command(
        &mut self,
        command: PaneCommand,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) {
        match command {
            PaneCommand::Split(orientation) => {
                if let Err(error) = self.split_pane(orientation, proxy, scheduler) {
                    warn!("Cannot split pane: {error}");
                    self.message_buffer.push(crate::message_bar::Message::new(
                        format!("Cannot split pane: {error}"),
                        crate::message_bar::MessageType::Warning,
                    ));
                    self.refresh_pane_sizes();
                }
            },
            PaneCommand::Close => {
                if self.panes.len() == 1 {
                    self.display.window.hold = false;
                }
                self.focused_pane().terminal.lock().exit();
            },
            PaneCommand::Focus(direction) => {
                if let Some(id) = self.geometry.neighbor(self.focused, direction) {
                    self.focus_pane(id, scheduler);
                }
            },
        }
    }

    fn split_pane(
        &mut self,
        orientation: Orientation,
        proxy: &EventLoopProxy<Event>,
        scheduler: &mut Scheduler,
    ) -> Result<(), Box<dyn Error>> {
        let id = self.allocate_pane_id();
        // Validate against a candidate tree before starting a shell or running its rc files.
        let params = self.display.layout_params(&self.config, &self.message_buffer);
        let mut candidate = self.layout.clone();
        let geometry = candidate
            .split(self.focused, id, orientation, params)
            .map_err(|error| format!("{error:?}"))?;
        let mut options = WindowOptions::default();
        #[cfg(not(windows))]
        {
            let focused = self.focused_pane();
            options.terminal_options.working_directory =
                crate::daemon::foreground_process_path(focused.master_fd, focused.shell_pid).ok();
        }
        if options.terminal_options.working_directory.is_none() {
            options.terminal_options.working_directory =
                self.config.general.working_directory.clone();
        }
        let view = self.display.pane_view_for_rect(geometry.panes[&id], &self.config);
        let pane = Pane::new(id, &self.config, &options, view, self.id(), proxy.clone())?;
        pane.terminal.lock().is_focused = false;
        self.layout = candidate;
        self.panes.insert(id, pane);
        self.focus_pane(id, scheduler);
        self.refresh_pane_sizes();
        self.display.window.request_redraw();
        Ok(())
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
            layout: Layout::Leaf(focused),
            geometry: LayoutGeometry {
                panes: [(focused, pane_view.rect)].into(),
                dividers: Vec::new(),
            },
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
            pointer: PointerState::default(),
            seed_pointer: false,
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

        let focused = self.panes.get_mut(&self.focused).expect("focused pane");
        self.display.process_renderer_update(&mut focused.view);
        let context_reset = self.display.begin_frame(&self.config);
        for (&id, pane) in &mut self.panes {
            if context_reset {
                pane.view.damage_tracker.frame().mark_fully_damaged();
            }
            if !pane.view.visual_bell.completed() {
                if self.display.window.has_frame {
                    self.display.window.request_redraw();
                } else {
                    self.dirty = true;
                }
            }
            let terminal = pane.terminal.lock();
            self.display.draw(
                &mut pane.view,
                terminal,
                &self.config,
                &mut pane.search_state,
                id == self.focused,
            );
        }
        let multiple_panes = self.panes.len() > 1;
        if multiple_panes {
            self.display.draw_pane_edges(
                self.geometry.dividers.iter().map(|divider| divider.rect),
                self.panes[&self.focused].view.rect,
                &self.config,
            );
        }
        let focused = self.panes.get_mut(&self.focused).expect("focused pane");
        self.display.finish_frame(
            &mut focused.view,
            scheduler,
            &self.message_buffer,
            &self.config,
            multiple_panes,
        );
        for pane in self.panes.values_mut() {
            pane.view.damage_tracker.swap_damage();
        }
    }

    /// Route window pointer events before acquiring a terminal lock.
    fn route_pointer_event(
        &mut self,
        mut event: WinitEvent<Event>,
        scheduler: &mut Scheduler,
    ) -> Option<WinitEvent<Event>> {
        if let WinitEvent::WindowEvent {
            event: WindowEvent::CursorMoved { position, .. }, ..
        } = &event
        {
            self.pointer.position = (position.x, position.y);
        }
        self.pointer_pane = hit_test(
            self.geometry.panes.iter().map(|(&id, &rect)| (id, rect)),
            self.pointer.position,
        );
        // Window messages remain clickable regardless of focused-pane origin or size.
        if self.pointer.capture.is_none() && self.pointer.drag.is_none() {
            if let Some(close) = self.display.message_bar_hit(
                &self.config,
                &self.message_buffer,
                self.pointer.position,
            ) {
                match &event {
                    WinitEvent::WindowEvent { event: WindowEvent::CursorMoved { .. }, .. } => {
                        self.display.window.set_mouse_cursor(if close {
                            CursorIcon::Pointer
                        } else {
                            CursorIcon::Default
                        });
                        return None;
                    },
                    WinitEvent::WindowEvent {
                        event: WindowEvent::MouseInput { button, state: ElementState::Pressed, .. },
                        ..
                    } => {
                        self.pointer.suppress(*button);
                        if close && *button == MouseButton::Left {
                            self.message_buffer.pop();
                            self.refresh_pane_sizes();
                        }
                        return None;
                    },
                    _ => (),
                }
            }
        }
        match &mut event {
            WinitEvent::WindowEvent {
                event: WindowEvent::CursorMoved { position, .. }, ..
            } => {
                self.pointer.position = (position.x, position.y);
                self.pointer_pane = hit_test(
                    self.geometry.panes.iter().map(|(&id, &rect)| (id, rect)),
                    self.pointer.position,
                );
                if let Some(drag) = self.pointer.drag.clone() {
                    let params = self.valid_layout_params();
                    match self.layout.drag_split(
                        &drag.path,
                        drag.position(self.pointer.position),
                        params,
                    ) {
                        Ok(_) => self.refresh_pane_sizes(),
                        Err(error) => {
                            warn!("Cannot drag pane divider: {error:?}");
                            self.pointer.drag = None;
                        },
                    }
                    self.update_pointer_cursor();
                    return None;
                }
                if self.pointer.capture.is_none() && self.pointer_pane != Some(self.focused) {
                    self.display.window.set_mouse_cursor(CursorIcon::Text);
                    self.update_pointer_cursor();
                    return None;
                }
            },
            WinitEvent::WindowEvent {
                window_id,
                event: WindowEvent::MouseInput { device_id, button, state, .. },
            } => {
                match self.pointer.button(*button, *state, self.focused, &self.geometry) {
                    ButtonAction::Forward => (),
                    ButtonAction::Ignore => {
                        self.update_pointer_cursor();
                        return None;
                    },
                    ButtonAction::Focus(id) => {
                        self.focus_pane(id, scheduler);
                        self.seed_pointer = true;
                        // Seed the newly focused pane's cell/selection coordinates without
                        // forwarding either edge of the focus click to its terminal.
                        event = WinitEvent::WindowEvent {
                            window_id: *window_id,
                            event: WindowEvent::CursorMoved {
                                device_id: *device_id,
                                position: PhysicalPosition::new(
                                    self.pointer.position.0,
                                    self.pointer.position.1,
                                ),
                            },
                        };
                    },
                }
            },
            WinitEvent::WindowEvent { event: WindowEvent::MouseWheel { .. }, .. }
                if self.pointer.drag.is_some() || self.pointer_pane != Some(self.focused) =>
            {
                return None;
            },
            _ => (),
        }
        translate_pointer_event(
            &mut event,
            self.focused_pane().view.rect,
            &self.geometry,
            &mut self.pointer.position,
            &mut self.pointer_pane,
        );
        Some(event)
    }

    fn update_pointer_cursor(&mut self) {
        if self.pointer.capture.is_some() {
            return;
        }
        let orientation = self
            .pointer
            .drag
            .as_ref()
            .map(|drag| drag.orientation)
            .or_else(|| self.pointer.divider_at(&self.geometry));
        if let Some(orientation) = orientation {
            let cursor = match orientation {
                Orientation::Vertical => CursorIcon::ColResize,
                Orientation::Horizontal => CursorIcon::RowResize,
            };
            self.display.window.set_mouse_cursor(cursor);
        }
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
            event => {
                self.event_queue.push(event);
                return;
            },
        }

        while !self.event_queue.is_empty() {
            let multiple_panes = self.panes.len() > 1;
            let pointer_routed = (multiple_panes || self.pointer.has_suppressed_buttons())
                && is_pointer_event(&self.event_queue[0]);
            if pointer_routed {
                let event = self.event_queue.remove(0);
                let Some(event) = self.route_pointer_event(event, scheduler) else { continue };
                self.event_queue.insert(0, event);
            }
            let next = &self.event_queue[0];
            let pane_id = event_target(next, self.focused);
            let count = if pointer_routed {
                1
            } else {
                self.event_queue
                    .iter()
                    .take_while(|event| {
                        event_target(event, self.focused) == pane_id
                            && !(multiple_panes && is_pointer_event(event))
                    })
                    .count()
            };
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

            let mut pane_commands = Vec::new();
            let context = ActionContext {
                pane_id,
                pane_commands: &mut pane_commands,
                pane_title: &mut pane.title,
                is_active_pane: pane_id == self.focused,
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

            let rect = processor.ctx.pane_view.rect;
            let remaining =
                crate::pane::process_batch(self.event_queue.drain(..count), |mut event| {
                    if !pointer_routed {
                        if let WinitEvent::WindowEvent {
                            event: WindowEvent::MouseInput { button, state, .. },
                            ..
                        } = &event
                        {
                            self.pointer.button(*button, *state, self.focused, &self.geometry);
                        }
                        translate_pointer_event(
                            &mut event,
                            rect,
                            &self.geometry,
                            &mut self.pointer.position,
                            &mut self.pointer_pane,
                        );
                    }
                    if std::mem::take(&mut self.seed_pointer) {
                        if let WinitEvent::WindowEvent {
                            event: WindowEvent::CursorMoved { position, .. },
                            ..
                        } = &event
                        {
                            processor.seed_mouse_position(*position);
                        } else {
                            processor.handle_event(event);
                        }
                    } else {
                        processor.handle_event(event);
                    }
                    !processor.ctx.pane_commands.is_empty()
                });
            self.event_queue.splice(0..0, remaining);

            // Shared font/window changes must reach every pane, not only this batch's target.
            if self.display.pending_update.dirty {
                self.submit_display_update(pane_id, old_is_searching, &mut terminal);
                self.dirty = true;
            }

            let pane = self.panes.get_mut(&pane_id).expect("event pane");
            if pane_id == self.focused && (self.dirty || self.mouse.hint_highlight_dirty) {
                self.dirty |= self.display.update_highlighted_hints(
                    &mut pane.view,
                    &terminal,
                    &self.config,
                    &self.mouse,
                    self.modifiers.state(),
                );
                self.mouse.hint_highlight_dirty = false;
            }
            drop(terminal);
            for command in pane_commands {
                self.apply_pane_command(command, event_proxy, scheduler);
            }
        }

        if self.panes.len() > 1 {
            self.update_pointer_cursor();
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

    /// Compute geometry once per update; rendering consumes the stored result.
    fn update_geometry(&mut self) {
        let mut params = self.display.layout_params(&self.config, &self.message_buffer);
        if self.panes.len() == 1 {
            // Preserve the upstream ability to shrink a single pane below its grid minimum.
            self.display.set_minimum_window_size(None);
            self.geometry = LayoutGeometry {
                panes: [(self.focused, params.bounds)].into(),
                dividers: Vec::new(),
            };
            return;
        }
        let minimum = match self.layout.minimum(params) {
            Ok(minimum) => minimum,
            Err(error) => {
                warn!("Cannot compute pane layout minimum: {error:?}");
                return;
            },
        };
        let message_height = self.display.window_size.height - params.bounds.height;
        self.display.set_minimum_window_size(Some(PhysicalSize::new(
            minimum.width,
            minimum.height.saturating_add(message_height),
        )));
        let clipped = params.bounds.width < minimum.width || params.bounds.height < minimum.height;
        params.bounds.width = params.bounds.width.max(minimum.width);
        params.bounds.height = params.bounds.height.max(minimum.height);
        match self.layout.geometry(params) {
            Ok(geometry) => self.geometry = geometry,
            Err(error) => warn!("Cannot update pane geometry: {error:?}"),
        }
        if clipped {
            self.display.resize_clipped_surface();
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
        self.update_geometry();
        for (&id, pane) in &mut self.panes {
            pane.view.rect = self.geometry.panes[&id];
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
                &MessageBuffer::default(),
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

/// Mouse policy changes can affect subsequent queued pointer events, so split those runs.
fn is_pointer_event(event: &WinitEvent<Event>) -> bool {
    matches!(event, WinitEvent::WindowEvent {
        event: WindowEvent::CursorMoved { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::Touch(_),
        ..
    })
}

fn translate_pointer_event(
    event: &mut WinitEvent<Event>,
    rect: PaneRect,
    geometry: &LayoutGeometry,
    pointer: &mut (f64, f64),
    pointer_pane: &mut Option<PaneId>,
) {
    let position = match event {
        WinitEvent::WindowEvent { event: WindowEvent::CursorMoved { position, .. }, .. } => {
            position
        },
        WinitEvent::WindowEvent { event: WindowEvent::Touch(touch), .. } => &mut touch.location,
        _ => return,
    };
    *pointer = (position.x, position.y);
    *pointer_pane = hit_test(geometry.panes.iter().map(|(&id, &rect)| (id, rect)), *pointer);
    let local = rect.local_position(*pointer);
    *position = PhysicalPosition::new(local.0, local.1);
}
