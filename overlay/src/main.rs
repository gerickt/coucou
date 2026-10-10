//! coucou-mochi: Mochi floating above the windows on the focused monitor.
//!
//!   coucou-mochi [--plugin-dir <dir>] [--managed]
//!   coucou-mochi --quit
//!
//! Noctalia's desktop widgets live on the Bottom layer, under every window;
//! this companion draws the same Mochi (the frames tools/render-mochi renders
//! from the original engine) on a layer-shell surface above them, as the
//! original Linux app did. The plugin's service (mochi/service.luau) starts
//! it with --managed when the `mochi_surface` setting says so and publishes
//! Mochi's state to $XDG_RUNTIME_DIR/coucou-mochi.json; this process draws,
//! reacts to the pointer and follows Hyprland's focused monitor.
//!
//! One instance per session ($XDG_RUNTIME_DIR/coucou-mochi.lock, exit code 3
//! when another runs). --managed: exits with its parent process and when the
//! service stops publishing (a plugin disabled).

mod app;
mod dropped;
mod gaze;
mod geometry;
mod hypr;
mod image;
mod manifest;
mod mochi;
mod player;
mod sound;
mod state;

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::{
    compositor::CompositorState,
    data_device_manager::DataDeviceManagerState,
    output::OutputState,
    reexports::{
        calloop::{
            generic::Generic,
            EventLoop, Interest, Mode, PostAction,
        },
        calloop_wayland_source::WaylandSource,
        client::{globals::registry_queue_init, Connection},
    },
    registry::RegistryState,
    seat::{pointer::cursor_shape::CursorShapeManager, SeatState},
    shell::wlr_layer::LayerShell,
    shm::Shm,
};

use app::App;

const EXIT_ALREADY_RUNNING: i32 = 3;
/// --managed: the service writes at least this often (heartbeat 20 s).
const STALE_AFTER: Duration = Duration::from_secs(75);

fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_absolute())
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
}

fn spots_path() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    state.join("coucou").join("overlay.json")
}

/// Where the plugin's frames and sounds are: --plugin-dir, $COUCOU_MOCHI_DIR,
/// the repository next to a development build, the Arch package's copy.
fn plugin_dir(arg: Option<PathBuf>) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.extend(arg);
    candidates.extend(std::env::var_os("COUCOU_MOCHI_DIR").map(PathBuf::from));
    if let Ok(exe) = std::env::current_exe() {
        // overlay/target/release/coucou-mochi -> mochi/
        if let Some(repo) = exe.ancestors().nth(4) {
            candidates.push(repo.join("mochi"));
        }
    }
    candidates.push(PathBuf::from("/usr/share/coucou/noctalia/mochi"));
    candidates.into_iter().find(|d| d.join("frames/manifest.json").is_file())
}

fn lock_path() -> PathBuf {
    runtime_dir().unwrap_or_else(|| PathBuf::from("/tmp")).join("coucou-mochi.lock")
}

/// Takes the session lock and writes our pid in it; None when another holds it.
fn take_lock() -> std::io::Result<Option<File>> {
    let mut f = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(lock_path())?;
    match f.try_lock() {
        Ok(()) => {
            f.set_len(0)?;
            write!(f, "{}", std::process::id())?;
            Ok(Some(f))
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// --quit: SIGTERM to the instance holding the lock.
fn quit_running() -> i32 {
    let Ok(mut f) = File::open(lock_path()) else { return 0 };
    if f.try_lock_shared().is_ok() {
        // Nobody holds it: nothing runs.
        return 0;
    }
    let mut text = String::new();
    let _ = f.read_to_string(&mut text);
    match text.trim().parse::<i32>() {
        Ok(pid) if pid > 1 => {
            // SAFETY: plain kill(2) on the pid the running instance wrote.
            unsafe { libc::kill(pid, libc::SIGTERM) };
            0
        }
        _ => 1,
    }
}

/// An inotify fd watching the runtime directory for the state file.
fn watch_dir(dir: &Path) -> Option<OwnedFd> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: plain syscalls; the fd is owned right away.
    unsafe {
        let fd = libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC);
        if fd < 0 {
            return None;
        }
        let fd = OwnedFd::from_raw_fd(fd);
        use std::os::fd::AsRawFd;
        if libc::inotify_add_watch(fd.as_raw_fd(), path.as_ptr(), libc::IN_CLOSE_WRITE | libc::IN_MOVED_TO) < 0 {
            return None;
        }
        Some(fd)
    }
}

/// A pidfd that becomes readable when the parent process exits.
fn parent_pidfd() -> Option<OwnedFd> {
    // SAFETY: getppid and pidfd_open are plain syscalls.
    unsafe {
        let ppid = libc::getppid();
        if ppid <= 1 {
            return None;
        }
        let fd = libc::syscall(libc::SYS_pidfd_open, ppid, 0) as i32;
        (fd >= 0).then(|| OwnedFd::from_raw_fd(fd))
    }
}

fn read_published(path: &Path) -> Option<state::Published> {
    std::fs::read_to_string(path).ok().and_then(|t| state::parse(&t))
}

fn usage() -> ! {
    eprintln!("usage: coucou-mochi [--plugin-dir <dir>] [--managed] | --quit");
    std::process::exit(2);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut dir_arg = None;
    let mut managed = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--plugin-dir" => dir_arg = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
            "--managed" => managed = true,
            "--quit" => std::process::exit(quit_running()),
            "--version" => {
                println!("coucou-mochi {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => usage(),
        }
    }
    let Some(plugin_dir) = plugin_dir(dir_arg) else {
        eprintln!("coucou-mochi: no plugin directory with frames/manifest.json (use --plugin-dir)");
        std::process::exit(1);
    };
    let _lock = match take_lock() {
        Ok(Some(f)) => f,
        Ok(None) => {
            eprintln!("coucou-mochi: already running");
            std::process::exit(EXIT_ALREADY_RUNNING);
        }
        Err(e) => {
            eprintln!("coucou-mochi: {}: {e}", lock_path().display());
            std::process::exit(1);
        }
    };
    if let Err(e) = run(plugin_dir, managed) {
        eprintln!("coucou-mochi: {e}");
        std::process::exit(1);
    }
}

fn run(plugin_dir: PathBuf, managed: bool) -> Result<(), String> {
    let text = std::fs::read_to_string(plugin_dir.join("frames/manifest.json")).map_err(|e| e.to_string())?;
    let manifest = manifest::parse(&text)?;
    if manifest.px == 0 || manifest.seqs.is_empty() {
        return Err("frames/manifest.json has no frames".into());
    }
    let runtime = runtime_dir().ok_or("XDG_RUNTIME_DIR is not set")?;
    let state_path = runtime.join(state::FILE_NAME);
    let published = read_published(&state_path).unwrap_or_default();
    if published.settings.surface == state::Surface::DesktopWidget {
        return Ok(());
    }

    let conn = Connection::connect_to_env().map_err(|e| format!("Wayland: {e}"))?;
    let (globals, queue) = registry_queue_init::<App>(&conn).map_err(|e| format!("Wayland: {e}"))?;
    let qh = queue.handle();
    let mut event_loop: EventLoop<'static, App> = EventLoop::try_new().map_err(|e| e.to_string())?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), queue).insert(handle.clone()).map_err(|e| e.to_string())?;

    let compositor = CompositorState::bind(&globals, &qh).map_err(|_| "no wl_compositor")?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| "the compositor has no wlr-layer-shell")?;
    let shm = Shm::bind(&globals, &qh).map_err(|_| "no wl_shm")?;
    let data_devices = DataDeviceManagerState::bind(&globals, &qh).ok();
    let cursor_shapes = CursorShapeManager::bind(&globals, &qh).ok();

    let player = player::Player::new(Arc::new(manifest.seqs));
    let mut mochi = mochi::Mochi::new(player, published.snap.clone(), published.settings.surface != state::Surface::Both);
    mochi.quiet_outfit_pop = published.settings.surface == state::Surface::Both;
    let hypr_dir = hypr::socket_dir();
    let mut app = App::new(
        RegistryState::new(&globals),
        SeatState::new(&globals, &qh),
        OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        data_devices,
        cursor_shapes,
        qh.clone(),
        handle.clone(),
        mochi,
        published,
        plugin_dir,
        spots_path(),
        hypr_dir.clone(),
    );

    // The service's state file.
    let mut last_write = Instant::now();
    if let Some(fd) = watch_dir(&runtime) {
        let path = state_path.clone();
        handle
            .insert_source(Generic::new(File::from(fd), Interest::READ, Mode::Level), move |_, f, app: &mut App| {
                // Drain the events; only ours matters.
                let mut buf = [0u8; 4096];
                let mut ours = false;
                // SAFETY: the file is only read, never dropped, while registered.
                let file: &mut File = unsafe { f.get_mut() };
                while let Ok(n) = file.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    let mut i = 0;
                    while i + 16 <= n {
                        let len = u32::from_ne_bytes(buf[i + 12..i + 16].try_into().unwrap()) as usize;
                        let name = &buf[i + 16..(i + 16 + len).min(n)];
                        let name = name.split(|b| *b == 0).next().unwrap_or(&[]);
                        if name == state::FILE_NAME.as_bytes() {
                            ours = true;
                        }
                        i += 16 + len;
                    }
                }
                if ours {
                    if let Some(p) = read_published(&path) {
                        app.set_published(p);
                    }
                }
                Ok(PostAction::Continue)
            })
            .map_err(|e| e.to_string())?;
    } else {
        eprintln!("coucou-mochi: no inotify, the state file is not followed");
    }

    // Hyprland: the focused monitor.
    if let Some(dir) = &hypr_dir {
        app.wanted_output = hypr::focused_monitor(dir);
        match UnixStream::connect(dir.join(".socket2.sock")) {
            Ok(stream) => {
                let _ = stream.set_nonblocking(true);
                let mut pending = String::new();
                handle
                    .insert_source(Generic::new(stream, Interest::READ, Mode::Level), move |_, s, app: &mut App| {
                        let mut buf = [0u8; 4096];
                        loop {
                            // SAFETY: the stream is only read, never dropped, while registered.
                            match unsafe { s.get_mut() }.read(&mut buf) {
                                Ok(0) => return Ok(PostAction::Remove),
                                Ok(n) => pending.push_str(&String::from_utf8_lossy(&buf[..n])),
                                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                                Err(_) => return Ok(PostAction::Remove),
                            }
                        }
                        for line in hypr::take_lines(&mut pending) {
                            if let Some(ev) = hypr::parse_event(&line) {
                                app.on_hypr(ev);
                            }
                        }
                        if pending.len() > 65536 {
                            pending.clear();
                        }
                        Ok(PostAction::Continue)
                    })
                    .map_err(|e| e.to_string())?;
            }
            Err(e) => eprintln!("coucou-mochi: Hyprland events: {e}"),
        }
    }

    // --managed: gone with the process that started us.
    if managed {
        if let Some(fd) = parent_pidfd() {
            handle
                .insert_source(Generic::new(File::from(fd), Interest::READ, Mode::Level), |_, _, app: &mut App| {
                    app.exit = true;
                    Ok(PostAction::Remove)
                })
                .map_err(|e| e.to_string())?;
        }
    }

    let mut last_ts = app.published.ts;
    let mut next_stale_check = Instant::now() + STALE_AFTER;
    while !app.exit {
        let mut timeout = app.timeout();
        if managed {
            let left = next_stale_check.saturating_duration_since(Instant::now());
            timeout = Some(timeout.map_or(left, |t| t.min(left)));
        }
        event_loop.dispatch(timeout, &mut app).map_err(|e| e.to_string())?;
        app.tick();
        if app.published.ts != last_ts {
            last_ts = app.published.ts;
            last_write = Instant::now();
        }
        if managed && Instant::now() >= next_stale_check {
            next_stale_check = Instant::now() + STALE_AFTER;
            if last_write.elapsed() > STALE_AFTER {
                eprintln!("coucou-mochi: the plugin's service stopped publishing, exiting");
                break;
            }
        }
    }
    Ok(())
}
