//! The Wayland side: one wlr-layer-shell surface on the Top layer (or
//! Overlay), anchored top-left with margins on the focused monitor, an input
//! region limited to Mochi's body so every other click goes to the windows
//! underneath, wl_shm double buffering, the pointer (hover, click, drag) and
//! file drops (wl_data_device, text/uri-list).
//!
//! The Top layer is the original's choice for the desktop Mochi
//! (platform/linux.rs prepare_desktop_window: "above windows, below
//! fullscreen video"): on Hyprland a fullscreen window covers him, and the
//! lock screen always does. Overlay (setting overlay_layer) also stays above
//! fullscreen windows.
//!
//! Exclusive zone -1, like the original: he reserves no space and his margins
//! count from the monitor's corner, the same space as Hyprland's cursor.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, FrameCallbackData, Region},
    data_device_manager::{
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        DataDeviceManagerState,
    },
    delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{LoopHandle, PostAction},
        client::{
            protocol::{wl_data_device::WlDataDevice, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
            Connection, QueueHandle,
        },
        protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        pointer::{cursor_shape::CursorShapeManager, PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
    },
};

use crate::geometry::{self, Size, Spot, Spots};
use crate::gaze;
use crate::hypr;
use crate::image::{self, Cache};
use crate::mochi::{Effect, Mochi};
use crate::sound::Sounds;
use crate::state::{LayerChoice, Published, Settings, Surface};

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
/// The eyes' cursor poll while he is awake and idle-ish (~8 Hz).
const POLL_EVERY: Duration = Duration::from_millis(125);
/// A frame callback this late: the surface is hidden (fullscreen window,
/// screens off); nothing animates until the compositor shows him again.
const STALLED_AFTER: Duration = Duration::from_millis(1000);
const FLIGHT: Duration = Duration::from_millis(420);
const PEEK_BACK_AFTER: Duration = Duration::from_millis(1200);
const PANEL: &str = "coucou/mochi:approval";
const PLUGIN: &str = "coucou/mochi";

/// Runs a command without waiting for it (reaped off the event loop).
pub fn spawn(argv: &[&str]) {
    use std::process::{Command, Stdio};
    let Some((cmd, args)) = argv.split_first() else { return };
    let child =
        Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    if let Ok(mut child) = child {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

struct Surf {
    layer: LayerSurface,
    output: wl_output::WlOutput,
    output_name: String,
    /// Monitor origin and size, logical px (the cursor's space).
    origin: (f64, f64),
    size: Size,
    configured: bool,
    scale: i32,
    pool: SlotPool,
    buffers: Vec<Buffer>,
    /// Logical side.
    side: u32,
    spot: (f64, f64),
    docked: bool,
    layer_choice: LayerChoice,
    drawn: Option<(String, u32)>,
    region_lift: Option<u32>,
}

struct Press {
    local: (f64, f64),
}

struct Flight {
    from: (f64, f64),
    to: (f64, f64),
    start: Instant,
}

pub struct App {
    pub registry_state: RegistryState,
    pub seat_state: SeatState,
    pub output_state: OutputState,
    pub compositor: CompositorState,
    pub layer_shell: LayerShell,
    pub shm: Shm,
    pub data_devices: Option<DataDeviceManagerState>,
    pub cursor_shapes: Option<CursorShapeManager>,
    pub qh: QueueHandle<App>,
    pub handle: LoopHandle<'static, App>,

    pointer: Option<wl_pointer::WlPointer>,
    shape_device: Option<WpCursorShapeDeviceV1>,
    data_device: Option<DataDevice>,
    surf: Option<Surf>,

    pub mochi: Mochi,
    pub published: Published,
    cache: Cache,
    sounds: Sounds,
    spots: Spots,
    spots_path: PathBuf,
    pub hypr: Option<PathBuf>,
    /// The monitor he should be on (Hyprland's focused one).
    pub wanted_output: Option<String>,

    press: Option<Press>,
    dragging: bool,
    grab: (f64, f64),
    drag_motion: Option<(f64, f64)>,
    hovering: bool,
    peek_out: bool,
    peek_back_at: Option<Instant>,
    flight: Option<Flight>,
    drop_buf: Vec<u8>,

    last_step: Instant,
    next_poll: Option<Instant>,
    frame_requested: Option<Instant>,
    needs_draw: bool,
    /// No surface (monitor gone, layer closed): place him again.
    pub needs_place: bool,
    /// COUCOU_MOCHI_DEBUG=1: pointer and drop events on stderr.
    debug: bool,
    pub exit: bool,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry_state: RegistryState,
        seat_state: SeatState,
        output_state: OutputState,
        compositor: CompositorState,
        layer_shell: LayerShell,
        shm: Shm,
        data_devices: Option<DataDeviceManagerState>,
        cursor_shapes: Option<CursorShapeManager>,
        qh: QueueHandle<App>,
        handle: LoopHandle<'static, App>,
        mochi: Mochi,
        published: Published,
        plugin_dir: PathBuf,
        spots_path: PathBuf,
        hypr: Option<PathBuf>,
    ) -> Self {
        let side = published.settings.size;
        App {
            registry_state,
            seat_state,
            output_state,
            compositor,
            layer_shell,
            shm,
            data_devices,
            cursor_shapes,
            qh,
            handle,
            pointer: None,
            shape_device: None,
            data_device: None,
            surf: None,
            mochi,
            published,
            cache: Cache::new(plugin_dir.clone(), side),
            sounds: Sounds::new(&plugin_dir),
            spots: Spots::load(&spots_path),
            spots_path,
            hypr,
            wanted_output: None,
            press: None,
            dragging: false,
            grab: (0.0, 0.0),
            drag_motion: None,
            hovering: false,
            peek_out: false,
            peek_back_at: None,
            flight: None,
            drop_buf: Vec::new(),
            last_step: Instant::now(),
            next_poll: None,
            frame_requested: None,
            needs_draw: true,
            needs_place: true,
            debug: std::env::var_os("COUCOU_MOCHI_DEBUG").is_some_and(|v| v == "1"),
            exit: false,
        }
    }

    fn settings(&self) -> &Settings {
        &self.published.settings
    }

    // ── Surface ───────────────────────────────────────────────────────────────

    fn output_named(&self, name: Option<&str>) -> Option<(wl_output::WlOutput, String, (f64, f64), Size)> {
        let mut first = None;
        for o in self.output_state.outputs() {
            let Some(info) = self.output_state.info(&o) else { continue };
            let oname = info.name.clone().unwrap_or_default();
            let origin = info.logical_position.unwrap_or(info.location);
            let size = info.logical_size.or_else(|| {
                info.modes.iter().find(|m| m.current).map(|m| {
                    let s = info.scale_factor.max(1);
                    (m.dimensions.0 / s, m.dimensions.1 / s)
                })
            });
            let Some(size) = size else { continue };
            let entry = (o.clone(), oname.clone(), (origin.0 as f64, origin.1 as f64), Size { w: size.0 as f64, h: size.1 as f64 });
            if name == Some(oname.as_str()) {
                return Some(entry);
            }
            if first.is_none() {
                first = Some(entry);
            }
        }
        first
    }

    /// (Re)creates his surface on the wanted monitor (or the first one).
    pub fn place(&mut self) {
        let Some((output, name, origin, size)) = self.output_named(self.wanted_output.as_deref()) else { return };
        let side = self.settings().size;
        let layer_choice = self.settings().layer;
        if let Some(s) = &self.surf {
            if s.output_name == name && s.side == side && s.layer_choice == layer_choice {
                return;
            }
        }
        self.surf = None;
        self.flight = None;
        let peek = self.settings().peek;
        let (spot, docked) = match self.spots.0.get(&name) {
            Some(s) if s.docked && peek => geometry::drop_spot((s.x, 0.0), side as f64, size, true),
            Some(s) => (geometry::clamp_spot((s.x, s.y), side as f64, size, geometry::MARGIN), false),
            None => (geometry::default_spot(side as f64, size), false),
        };
        let surface = self.compositor.create_surface(&self.qh);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            surface,
            if layer_choice == LayerChoice::Overlay { Layer::Overlay } else { Layer::Top },
            Some("coucou-mochi"),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        layer.set_size(side, side);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_margin(spot.1.round() as i32, 0, 0, spot.0.round() as i32);
        // No input until the first configure sets his body as the region.
        if let Ok(region) = Region::new(&self.compositor) {
            layer.wl_surface().set_input_region(Some(region.wl_region()));
        }
        layer.commit();
        let bytes = (side * side * 4 * 2) as usize;
        let Ok(pool) = SlotPool::new(bytes, &self.shm) else {
            eprintln!("coucou-mochi: no shared memory pool");
            return;
        };
        self.peek_out = false;
        self.surf = Some(Surf {
            layer,
            output,
            output_name: name,
            origin,
            size,
            configured: false,
            scale: 1,
            pool,
            buffers: Vec::new(),
            side,
            spot,
            docked,
            layer_choice,
            drawn: None,
            region_lift: None,
        });
        self.frame_requested = None;
        self.needs_draw = true;
    }

    /// How far he is drawn above the surface's top: docked and not peeking out.
    fn lift(&self) -> u32 {
        match &self.surf {
            Some(s) if s.docked && !self.peek_out => (s.side as f64 * geometry::PEEK_HIDDEN).round() as u32,
            _ => 0,
        }
    }

    fn apply_region(&mut self) {
        let lift = self.lift();
        let Some(s) = &mut self.surf else { return };
        if s.region_lift == Some(lift) || !s.configured {
            return;
        }
        let Ok(region) = Region::new(&self.compositor) else { return };
        for (x, y, w, h) in geometry::body_rows(s.side as f64, lift as f64) {
            region.add(x, y, w, h);
        }
        s.layer.wl_surface().set_input_region(Some(region.wl_region()));
        s.region_lift = Some(lift);
        s.layer.wl_surface().commit();
    }

    fn move_to(&mut self, spot: (f64, f64)) {
        let Some(s) = &mut self.surf else { return };
        s.spot = spot;
        s.layer.set_margin(spot.1.round() as i32, 0, 0, spot.0.round() as i32);
        s.layer.wl_surface().commit();
    }

    fn save_spot(&mut self) {
        let Some(s) = &self.surf else { return };
        self.spots.0.insert(s.output_name.clone(), Spot { x: s.spot.0, y: s.spot.1, docked: s.docked });
        if let Err(e) = self.spots.save(&self.spots_path) {
            eprintln!("coucou-mochi: {}: {e}", self.spots_path.display());
        }
    }

    fn draw(&mut self) {
        let lift = self.lift();
        let Some(file) = self.mochi.frame() else { return };
        let Some(s) = &mut self.surf else { return };
        if !s.configured {
            return;
        }
        if s.drawn.as_ref() == Some(&(file.clone(), lift)) {
            self.needs_draw = false;
            return;
        }
        let px = s.side * s.scale as u32;
        self.cache.set_side(px);
        let Some(frame) = self.cache.get(&file) else { return };
        let stride = px as i32 * 4;
        // Double buffer: a free one of the two, else a new one.
        let mut pick = None;
        for (i, b) in s.buffers.iter().enumerate() {
            if b.height() == px as i32 && b.canvas(&mut s.pool).is_some() {
                pick = Some(i);
                break;
            }
        }
        let i = match pick {
            Some(i) => i,
            None => {
                let Ok((b, _)) = s.pool.create_buffer(px as i32, px as i32, stride, wl_shm::Format::Argb8888) else {
                    return;
                };
                if s.buffers.len() >= 2 {
                    s.buffers.remove(0);
                }
                s.buffers.push(b);
                s.buffers.len() - 1
            }
        };
        let Some(canvas) = s.buffers[i].canvas(&mut s.pool) else { return };
        image::blit(canvas, px, frame, lift * s.scale as u32);
        let surface = s.layer.wl_surface();
        surface.set_buffer_scale(s.scale);
        surface.damage_buffer(0, 0, px as i32, px as i32);
        surface.frame(&self.qh, FrameCallbackData(surface.clone()));
        if s.buffers[i].attach_to(surface).is_err() {
            return;
        }
        s.layer.commit();
        s.drawn = Some((file, lift));
        self.frame_requested = Some(Instant::now());
        self.needs_draw = false;
    }

    fn stalled(&self) -> bool {
        self.frame_requested.is_some_and(|t| t.elapsed() > STALLED_AFTER)
    }

    // ── Clock ─────────────────────────────────────────────────────────────────

    fn polling(&self) -> bool {
        self.hypr.is_some()
            && self.settings().follow_cursor
            && self.surf.as_ref().is_some_and(|s| s.configured)
            && !self.stalled()
            && !self.dragging
            && self.mochi.wants_gaze()
    }

    /// How long the event loop may sleep (None: until an event).
    pub fn timeout(&self) -> Option<Duration> {
        let now = Instant::now();
        let mut due: Option<Duration> = None;
        if self.needs_place {
            return Some(Duration::ZERO);
        }
        let mut at = |d: Duration| due = Some(due.map_or(d, |x| x.min(d)));
        if self.flight.is_some() {
            at(Duration::from_millis(16));
        }
        if self.dragging && self.drag_motion.is_some() {
            at(Duration::ZERO);
        }
        if let Some(t) = self.peek_back_at {
            at(t.saturating_duration_since(now));
        }
        // Hidden: nothing to animate until the compositor shows him again.
        if !self.stalled() {
            let d = self.mochi.due();
            if d.is_finite() {
                at(Duration::from_secs_f64(d.min(3600.0)));
            }
            if self.polling() {
                at(self.next_poll.map_or(Duration::ZERO, |t| t.saturating_duration_since(now)));
            }
        }
        due
    }

    pub fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_step).as_secs_f64();
        self.last_step = now;

        if self.needs_place {
            self.needs_place = false;
            self.place();
        }
        if self.dragging {
            self.drag_step();
        }
        if let Some(f) = &self.flight {
            let p = f.start.elapsed().as_secs_f64() / FLIGHT.as_secs_f64();
            let spot = geometry::flight(f.from, f.to, p);
            let done = p >= 1.0;
            self.move_to(spot);
            if done {
                self.flight = None;
                self.save_spot();
            }
        }
        if self.peek_back_at.is_some_and(|t| now >= t) {
            self.peek_back_at = None;
            if !self.hovering {
                self.peek_out = false;
                self.needs_draw = true;
                self.apply_region();
            }
        }
        if self.polling() && self.next_poll.is_none_or(|t| now >= t) {
            self.next_poll = Some(now + POLL_EVERY);
            self.poll_cursor();
        } else if !self.polling() {
            self.next_poll = None;
        }

        if self.mochi.step(dt) {
            self.needs_draw = true;
        }
        self.run_effects();
        if self.needs_draw && self.frame_requested.is_none() {
            self.draw();
        }
    }

    fn poll_cursor(&mut self) {
        let (Some(dir), Some(s)) = (&self.hypr, &self.surf) else { return };
        let Some(cursor) = hypr::cursor(dir) else { return };
        let half = s.side as f64 / 2.0;
        let center = (s.origin.0 + s.spot.0 + half, s.origin.1 + s.spot.1 + half - self.lift() as f64);
        let distance = (cursor.0 - center.0).hypot(cursor.1 - center.1);
        let look = gaze::direction(gaze::gaze(center, cursor));
        self.mochi.set_cursor(Some(distance), look);
    }

    fn run_effects(&mut self) {
        let effects: Vec<Effect> = std::mem::take(&mut self.mochi.effects);
        for e in effects {
            match e {
                Effect::Sound(name) => {
                    if self.settings().sound {
                        self.sounds.play(name);
                    }
                }
                Effect::OpenApproval => spawn(&["noctalia", "msg", "panel-open", PANEL]),
                Effect::Home => self.fly_home(),
            }
        }
    }

    fn fly_home(&mut self) {
        let Some(s) = &mut self.surf else { return };
        let home = geometry::default_spot(s.side as f64, s.size);
        s.docked = false;
        self.peek_out = false;
        self.flight = Some(Flight { from: s.spot, to: home, start: Instant::now() });
        self.needs_draw = true;
        self.apply_region();
        if self.settings().sound {
            self.sounds.play("peek");
        }
    }

    // ── Published state ───────────────────────────────────────────────────────

    pub fn set_published(&mut self, p: Published) {
        if p.settings.surface == Surface::DesktopWidget {
            // The desktop widget alone: the service stops us too.
            self.exit = true;
            return;
        }
        let replace = p.settings.size != self.published.settings.size || p.settings.layer != self.published.settings.layer;
        self.mochi.quiet_outfit_pop = p.settings.surface == Surface::Both;
        self.mochi.set_snapshot(p.snap.clone());
        self.published = p;
        if replace {
            self.place();
        }
        self.needs_draw = true;
    }

    pub fn on_hypr(&mut self, ev: hypr::Event) {
        match ev {
            hypr::Event::FocusedMonitor(name) => {
                self.wanted_output = Some(name);
                if !self.dragging {
                    self.place();
                }
            }
            hypr::Event::MonitorRemoved(name) => {
                if self.surf.as_ref().is_some_and(|s| s.output_name == name) {
                    self.surf = None;
                    self.needs_place = true;
                }
            }
            hypr::Event::MonitorAdded(_) => {}
        }
    }

    // ── Pointer ───────────────────────────────────────────────────────────────

    fn begin_drag(&mut self, grab: (f64, f64)) {
        self.dragging = true;
        self.grab = grab;
        self.flight = None;
        self.mochi.set_dragging(true);
        let lift = self.lift() as f64;
        if let Some(s) = &mut self.surf {
            // Out of the dock while he is carried: drawn whole again, so the
            // grabbed point moves down by what was hidden.
            if s.docked {
                s.docked = false;
                self.grab.1 += lift;
            }
        }
        self.peek_out = false;
        self.needs_draw = true;
        self.apply_region();
    }

    fn drag_step(&mut self) {
        let Some(local) = self.drag_motion.take() else { return };
        let Some(s) = &self.surf else { return };
        let spot = match self.hypr.as_ref().and_then(|d| hypr::cursor(d)) {
            Some(cursor) => geometry::drag_spot(cursor, self.grab, s.origin),
            // Without a global cursor: surface-local deltas.
            None => (s.spot.0 + local.0 - self.grab.0, s.spot.1 + local.1 - self.grab.1),
        };
        self.move_to(spot);
    }

    fn end_drag(&mut self) {
        self.dragging = false;
        self.drag_step();
        self.mochi.set_dragging(false);
        let peek = self.settings().peek;
        let Some(s) = &mut self.surf else { return };
        let (spot, docked) = geometry::drop_spot(s.spot, s.side as f64, s.size, peek);
        s.docked = docked;
        self.peek_out = docked && self.hovering;
        self.move_to(spot);
        self.save_spot();
        self.needs_draw = true;
        self.apply_region();
    }

    fn on_pointer(&mut self, ev: &PointerEvent) {
        let Some(s) = &self.surf else { return };
        if &ev.surface != s.layer.wl_surface() {
            return;
        }
        let side = s.side as f64;
        let docked = s.docked;
        let lift = self.lift() as f64;
        // In his own frame (the picture is drawn `lift` px higher when docked).
        let local = (ev.position.0, ev.position.1 + lift);
        if self.debug && !matches!(ev.kind, PointerEventKind::Motion { .. }) {
            eprintln!("coucou-mochi: pointer {:?} at {:.0},{:.0}", ev.kind, ev.position.0, ev.position.1);
        }
        match ev.kind {
            PointerEventKind::Enter { serial } => {
                if let Some(dev) = &self.shape_device {
                    dev.set_shape(serial, Shape::Default);
                }
                self.hovering = true;
                self.peek_back_at = None;
                self.mochi.hover(true);
                if docked && !self.peek_out {
                    // The island's peek: he slides out of the top edge.
                    self.peek_out = true;
                    self.needs_draw = true;
                    self.apply_region();
                    if self.settings().sound {
                        self.sounds.play("peek");
                    }
                    if self.settings().open_on_hover {
                        spawn(&["noctalia", "msg", "panel-open", PANEL]);
                    }
                }
            }
            PointerEventKind::Leave { .. } => {
                self.hovering = false;
                self.mochi.hover(false);
                if docked && !self.dragging {
                    self.peek_back_at = Some(Instant::now() + PEEK_BACK_AFTER);
                }
            }
            PointerEventKind::Motion { .. } => {
                if self.dragging {
                    self.drag_motion = Some(ev.position);
                    return;
                }
                self.mochi.pointer_moved();
                if let Some(p) = &self.press {
                    if (local.0 - p.local.0).hypot(local.1 - p.local.1) > geometry::DRAG_THRESHOLD {
                        let grab = (p.local.0, p.local.1 - lift);
                        self.begin_drag(grab);
                        self.drag_motion = Some(ev.position);
                    }
                } else if self.hypr.is_none() && self.settings().follow_cursor {
                    // No global cursor: the eyes follow the pointer over him
                    // (the original Linux window's rule).
                    let c = side / 2.0;
                    self.mochi.set_cursor(None, gaze::direction(gaze::gaze((c, c), local)));
                }
            }
            PointerEventKind::Press { button, .. } => {
                if button == BTN_LEFT && geometry::is_over_body(local, side) {
                    self.press = Some(Press { local });
                } else if button == BTN_RIGHT && geometry::is_over_body(local, side) {
                    // Right click: the plugin's settings (outfit, sounds, overlay).
                    spawn(&["noctalia", "msg", "settings-open-plugin", PLUGIN]);
                }
            }
            PointerEventKind::Release { button, .. } => {
                if button != BTN_LEFT {
                    return;
                }
                let press = self.press.take();
                if self.dragging {
                    self.end_drag();
                } else if press.is_some() {
                    self.mochi.click();
                }
            }
            PointerEventKind::Axis { .. } => {}
        }
    }

    // ── Drop ──────────────────────────────────────────────────────────────────

    fn on_drop_data(&mut self, data: &[u8]) {
        let text = String::from_utf8_lossy(data);
        let Some(path) = crate::dropped::local_paths(&text).into_iter().find(|p| p.is_file()) else { return };
        let Some(path) = path.to_str() else { return };
        if self.debug {
            eprintln!("coucou-mochi: dropped {path}");
        }
        // The service resets the chat around the file, opens it and makes him
        // swallow it (the gulp comes back through the published reaction).
        spawn(&["noctalia", "msg", "plugin", "coucou/mochi:daemon", "all", "attach", path]);
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

impl CompositorHandler for App {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, factor: i32) {
        if let Some(s) = &mut self.surf {
            if s.layer.wl_surface() == surface && s.scale != factor.max(1) {
                s.scale = factor.max(1);
                s.drawn = None;
                self.needs_draw = true;
            }
        }
    }

    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_requested = None;
    }

    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}

    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        if self.surf.is_none() {
            self.needs_place = true;
        }
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        // A new mode or position: clamp him into the monitor again.
        let Some(info) = self.output_state.info(&output) else { return };
        let Some(s) = &mut self.surf else { return };
        if s.output != output {
            return;
        }
        if let Some(pos) = info.logical_position {
            s.origin = (pos.0 as f64, pos.1 as f64);
        }
        if let Some(size) = info.logical_size {
            s.size = Size { w: size.0 as f64, h: size.1 as f64 };
            let (spot, side, docked) = (s.spot, s.side as f64, s.docked);
            if !docked {
                let clamped = geometry::clamp_spot(spot, side, s.size, geometry::MARGIN);
                if clamped != spot {
                    self.move_to(clamped);
                }
            }
        }
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if self.surf.as_ref().is_some_and(|s| s.output == output) {
            self.surf = None;
            self.needs_place = true;
        }
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.surf.as_ref().is_some_and(|s| &s.layer == layer) {
            self.surf = None;
            self.needs_place = true;
        }
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, _: LayerSurfaceConfigure, _: u32) {
        let Some(s) = &mut self.surf else { return };
        if &s.layer != layer {
            return;
        }
        s.configured = true;
        s.drawn = None;
        s.region_lift = None;
        self.frame_requested = None;
        self.needs_draw = true;
        self.apply_region();
        self.draw();
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            if let Ok(pointer) = self.seat_state.get_pointer(qh, &seat) {
                if let Some(shapes) = &self.cursor_shapes {
                    self.shape_device = Some(shapes.get_shape_device(&pointer, qh));
                }
                self.pointer = Some(pointer);
            }
            if self.data_device.is_none() {
                if let Some(m) = &self.data_devices {
                    self.data_device = Some(m.get_data_device(qh, &seat));
                }
            }
        }
    }

    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            if let Some(dev) = self.shape_device.take() {
                dev.destroy();
            }
            if let Some(p) = self.pointer.take() {
                p.release();
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for App {
    fn pointer_frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        for ev in events {
            self.on_pointer(ev);
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

fn uri_offer(offer: &DragOffer) -> bool {
    offer.with_mime_types(|m| m.iter().any(|t| t == "text/uri-list"))
}

impl DataDeviceHandler for App {
    fn enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64, _: &wl_surface::WlSurface) {
        let Some(offer) = self.data_device.as_ref().and_then(|d| d.data().drag_offer()) else { return };
        if uri_offer(&offer) {
            offer.accept_mime_type(offer.serial, Some("text/uri-list".into()));
            offer.set_actions(DndAction::Copy, DndAction::Copy);
            self.mochi.hover(true);
        } else {
            offer.accept_mime_type(offer.serial, None);
        }
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        if !self.hovering {
            self.mochi.hover(false);
        }
    }

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}

    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, conn: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        let Some(offer) = self.data_device.as_ref().and_then(|d| d.data().drag_offer()) else { return };
        if !uri_offer(&offer) {
            return;
        }
        let Ok(pipe) = offer.receive("text/uri-list".into()) else {
            offer.finish();
            return;
        };
        let _ = conn.flush();
        self.drop_buf.clear();
        let inserted = self.handle.insert_source(pipe, move |_, f, app: &mut App| {
            use std::io::Read;
            // SAFETY: the file is only read, never dropped, while registered.
            let f: &mut std::fs::File = unsafe { f.get_mut() };
            let mut chunk = [0u8; 4096];
            loop {
                match f.read(&mut chunk) {
                    Ok(0) => {
                        let data = std::mem::take(&mut app.drop_buf);
                        offer.finish();
                        offer.destroy();
                        app.on_drop_data(&data);
                        return PostAction::Remove;
                    }
                    Ok(n) => {
                        app.drop_buf.extend_from_slice(&chunk[..n]);
                        // A path list, not a file: cap it.
                        if app.drop_buf.len() > 64 * 1024 {
                            app.drop_buf.clear();
                            offer.finish();
                            offer.destroy();
                            return PostAction::Remove;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return PostAction::Continue,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        offer.finish();
                        offer.destroy();
                        return PostAction::Remove;
                    }
                }
            }
        });
        if inserted.is_err() {
            eprintln!("coucou-mochi: could not read the drop");
        }
    }
}

use smithay_client_toolkit::reexports::client::protocol::wl_data_device_manager::DndAction;

impl DataOfferHandler for App {
    fn source_actions(&mut self, _: &Connection, _: &QueueHandle<Self>, offer: &mut DragOffer, _: DndAction) {
        offer.set_actions(DndAction::Copy, DndAction::Copy);
    }

    fn selected_action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &mut DragOffer, _: DndAction) {}
}

delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(App);
