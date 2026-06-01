use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::time::Instant;
use std::sync::atomic::{AtomicI32, Ordering};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::{sleep, Duration};
use crate::config::InertialConfig;

pub static POINTER_X: AtomicI32 = AtomicI32::new(0);
pub static POINTER_Y: AtomicI32 = AtomicI32::new(0);
pub static SCREEN_WIDTH: AtomicI32 = AtomicI32::new(1920);
pub static SCREEN_HEIGHT: AtomicI32 = AtomicI32::new(1080);

pub fn update_pointer_coords(dx: i32, dy: i32) {
    let screen_w = SCREEN_WIDTH.load(Ordering::SeqCst);
    let screen_h = SCREEN_HEIGHT.load(Ordering::SeqCst);
    let mut current_x = POINTER_X.load(Ordering::SeqCst);
    let mut current_y = POINTER_Y.load(Ordering::SeqCst);

    current_x = (current_x + dx).clamp(0, screen_w);
    current_y = (current_y + dy).clamp(0, screen_h);

    POINTER_X.store(current_x, Ordering::SeqCst);
    POINTER_Y.store(current_y, Ordering::SeqCst);
}

// IOCTL and Event constants
const UI_DEV_CREATE: libc::c_ulong = 0x5501;
const UI_DEV_SETUP: libc::c_ulong = 0x405C5503;
const UI_SET_EVBIT: libc::c_ulong = 0x40045564;
const UI_SET_RELBIT: libc::c_ulong = 0x40045566;
const UI_SET_KEYBIT: libc::c_ulong = 0x40045565;

// Linux input event codes
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;

const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;

const BTN_TOUCH: u16 = 0x14a;

const SYN_REPORT: u16 = 0x00;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct UinputSetup {
    id: InputId,
    name: [u8; 80],
    ff_effects_max: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    type_: u16,
    code: u16,
    value: i32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FingerState {
    pub slot: usize,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct SlotState {
    active: bool,
    x: f32,
    y: f32,
    raw_x: Option<i32>,
    raw_y: Option<i32>,
}

fn eviocgabs(abs: u32) -> libc::c_ulong {
    let size = std::mem::size_of::<libc::input_absinfo>() as u64;
    ((2u64 << 30) | ((b'E' as u64) << 8) | ((0x40 + abs) as u64) | (size << 16)) as libc::c_ulong
}

const ABS_MT_SLOT: u16 = 0x2f;

#[derive(Debug, Clone)]
pub enum InputDaemonMsg {
    UpdateConfig(InertialConfig, bool),
    SimulateMove { dx: i32, dy: i32 },
    SimulateButton { button: u16, press: bool },
    SimulateKey { keycode: u16, press: bool },
    SimulateClick { button: u16 },
    SimulateKeyPress { keycode: u16 },
}

#[derive(Debug)]
enum CoordinatorMsg {
    PhysicalMove { dx: i32, dy: i32, timestamp: Instant },
    PhysicalTrackpadMove { dx: i32, dy: i32, timestamp: Instant },
    PhysicalTrackpadLift { timestamp: Instant },
    PhysicalScroll { dwx: i32, dwy: i32, timestamp: Instant },
    FingersReport(Vec<FingerState>),
    DaemonMsg(InputDaemonMsg),
    InternalReleaseButton { button: u16 },
    InternalReleaseKey { keycode: u16 },
}

fn setup_uinput() -> std::io::Result<std::fs::File> {
    let file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open("/dev/uinput")?;

    let fd = file.as_raw_fd();

    unsafe {
        if libc::ioctl(fd, UI_SET_EVBIT, EV_REL as libc::c_int) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::ioctl(fd, UI_SET_RELBIT, REL_X as libc::c_int) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::ioctl(fd, UI_SET_RELBIT, REL_Y as libc::c_int) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::ioctl(fd, UI_SET_RELBIT, REL_WHEEL as libc::c_int) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::ioctl(fd, UI_SET_RELBIT, REL_HWHEEL as libc::c_int) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::ioctl(fd, UI_SET_EVBIT, EV_KEY as libc::c_int) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        for key in 1..=511 {
            if libc::ioctl(fd, UI_SET_KEYBIT, key as libc::c_int) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        for btn in 272..=276 {
            if libc::ioctl(fd, UI_SET_KEYBIT, btn as libc::c_int) < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
    }

    let mut setup = UinputSetup {
        id: InputId {
            bustype: 0x0006, // BUS_VIRTUAL
            vendor: 0x1234,
            product: 0x5678,
            version: 1,
        },
        name: [0; 80],
        ff_effects_max: 0,
    };

    let name_bytes = b"Clear Virtual Mouse";
    setup.name[..name_bytes.len()].copy_from_slice(name_bytes);

    unsafe {
        let setup_ptr = &setup as *const UinputSetup as *const libc::c_void;
        if libc::ioctl(fd, UI_DEV_SETUP, setup_ptr) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::ioctl(fd, UI_DEV_CREATE) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }

    println!("[input-subsystem] Successfully created virtual uinput device.");
    Ok(file)
}

fn write_raw_event(file: &mut std::fs::File, type_: u16, code: u16, value: i32) -> std::io::Result<()> {
    let event = InputEvent {
        tv_sec: 0,
        tv_usec: 0,
        type_,
        code,
        value,
    };
    let buf = unsafe {
        std::slice::from_raw_parts(&event as *const InputEvent as *const u8, std::mem::size_of::<InputEvent>())
    };
    use std::io::Write;
    file.write_all(buf)?;
    file.flush()?;
    Ok(())
}

fn write_mouse_move(file: &mut std::fs::File, dx: i32, dy: i32) -> std::io::Result<()> {
    if dx != 0 {
        write_raw_event(file, EV_REL, REL_X, dx)?;
    }
    if dy != 0 {
        write_raw_event(file, EV_REL, REL_Y, dy)?;
    }
    write_raw_event(file, EV_SYN, SYN_REPORT, 0)?;
    Ok(())
}

fn write_scroll(file: &mut std::fs::File, dwx: i32, dwy: i32) -> std::io::Result<()> {
    if dwx != 0 {
        write_raw_event(file, EV_REL, REL_HWHEEL, dwx)?;
    }
    if dwy != 0 {
        write_raw_event(file, EV_REL, REL_WHEEL, dwy)?;
    }
    write_raw_event(file, EV_SYN, SYN_REPORT, 0)?;
    Ok(())
}

struct TouchTracker {
    dx: i32,
    dy: i32,
    finger_down: bool,
}

async fn read_device_loop(
    path: PathBuf,
    tx: UnboundedSender<CoordinatorMsg>,
) -> std::io::Result<()> {
    println!("[input-subsystem] Started monitoring {:?}", path);
    let mut file = tokio::fs::File::open(&path).await?;
    let fd = file.as_raw_fd();

    let mut min_x = 0;
    let mut max_x = 3528;
    let mut min_y = 0;
    let mut max_y = 2006;

    let mut absinfo = libc::input_absinfo {
        value: 0, minimum: 0, maximum: 0, fuzz: 0, flat: 0, resolution: 0
    };
    unsafe {
        if libc::ioctl(fd, eviocgabs(ABS_MT_POSITION_X as u32), &mut absinfo) >= 0 {
            min_x = absinfo.minimum;
            max_x = absinfo.maximum;
        } else if libc::ioctl(fd, eviocgabs(ABS_X as u32), &mut absinfo) >= 0 {
            min_x = absinfo.minimum;
            max_x = absinfo.maximum;
        }
        if libc::ioctl(fd, eviocgabs(ABS_MT_POSITION_Y as u32), &mut absinfo) >= 0 {
            min_y = absinfo.minimum;
            max_y = absinfo.maximum;
        } else if libc::ioctl(fd, eviocgabs(ABS_Y as u32), &mut absinfo) >= 0 {
            min_y = absinfo.minimum;
            max_y = absinfo.maximum;
        }
    }

    let range_x = (max_x - min_x).max(1) as f32;
    let range_y = (max_y - min_y).max(1) as f32;

    let mut slots = [SlotState::default(); 16];
    let mut current_slot = 0usize;
    let mut buf = [0u8; 24];

    let mut touch = TouchTracker {
        dx: 0,
        dy: 0,
        finger_down: false,
    };

    loop {
        file.read_exact(&mut buf).await?;
        let event: InputEvent = unsafe { std::ptr::read_unaligned(buf.as_ptr() as *const InputEvent) };
        let now = Instant::now();

        if event.type_ == EV_REL {
            if event.code == REL_X {
                let _ = tx.send(CoordinatorMsg::PhysicalMove { dx: event.value, dy: 0, timestamp: now });
            } else if event.code == REL_Y {
                let _ = tx.send(CoordinatorMsg::PhysicalMove { dx: 0, dy: event.value, timestamp: now });
            } else if event.code == REL_WHEEL {
                let _ = tx.send(CoordinatorMsg::PhysicalScroll { dwx: 0, dwy: event.value, timestamp: now });
            } else if event.code == REL_HWHEEL {
                let _ = tx.send(CoordinatorMsg::PhysicalScroll { dwx: event.value, dwy: 0, timestamp: now });
            }
        } else if event.type_ == EV_ABS {
            if event.code == ABS_X || event.code == ABS_MT_POSITION_X {
                let val = event.value;
                let slot_idx = if event.code == ABS_X { 0 } else { current_slot };
                let is_primary = slots.iter().position(|s| s.active) == Some(slot_idx);
                let slot = &mut slots[slot_idx];
                if is_primary {
                    if let Some(lx) = slot.raw_x {
                        touch.dx += val - lx;
                    }
                }
                slot.raw_x = Some(val);
                slot.x = (val - min_x) as f32 / range_x;
            } else if event.code == ABS_Y || event.code == ABS_MT_POSITION_Y {
                let val = event.value;
                let slot_idx = if event.code == ABS_Y { 0 } else { current_slot };
                let is_primary = slots.iter().position(|s| s.active) == Some(slot_idx);
                let slot = &mut slots[slot_idx];
                if is_primary {
                    if let Some(ly) = slot.raw_y {
                        touch.dy += val - ly;
                    }
                }
                slot.raw_y = Some(val);
                slot.y = (val - min_y) as f32 / range_y;
            } else if event.code == ABS_MT_TRACKING_ID {
                if event.value >= 0 {
                    slots[current_slot].active = true;
                    touch.finger_down = true;
                } else {
                    slots[current_slot].active = false;
                    slots[current_slot].raw_x = None;
                    slots[current_slot].raw_y = None;
                    touch.finger_down = slots.iter().any(|s| s.active);
                }
            } else if event.code == ABS_MT_SLOT {
                current_slot = (event.value as usize).min(15);
            }
        } else if event.type_ == EV_KEY {
            if event.code == BTN_TOUCH {
                if event.value == 1 {
                    touch.finger_down = true;
                    slots[0].active = true;
                } else {
                    touch.finger_down = false;
                    for s in &mut slots {
                        s.active = false;
                        s.raw_x = None;
                        s.raw_y = None;
                    }
                }
            }
        } else if event.type_ == EV_SYN {
            if event.code == SYN_REPORT {
                let active_fingers: Vec<FingerState> = slots.iter().enumerate()
                    .filter(|(_, slot)| slot.active)
                    .map(|(i, slot)| FingerState { slot: i, x: slot.x, y: slot.y })
                    .collect();
                let _ = tx.send(CoordinatorMsg::FingersReport(active_fingers));

                if touch.finger_down {
                    if touch.dx != 0 || touch.dy != 0 {
                        let scaled_dx = (touch.dx as f32 * 0.15) as i32;
                        let scaled_dy = (touch.dy as f32 * 0.15) as i32;
                        if scaled_dx != 0 || scaled_dy != 0 {
                            let _ = tx.send(CoordinatorMsg::PhysicalTrackpadMove { dx: scaled_dx, dy: scaled_dy, timestamp: now });
                        }
                        touch.dx = 0;
                        touch.dy = 0;
                    }
                } else {
                    let _ = tx.send(CoordinatorMsg::PhysicalTrackpadLift { timestamp: now });
                    touch.dx = 0;
                    touch.dy = 0;
                    for s in &mut slots {
                        s.raw_x = None;
                        s.raw_y = None;
                    }
                }
            }
        }
    }
}

struct PhysicsState {
    config: InertialConfig,

    last_move_time: Instant,
    vel_x: f32,
    vel_y: f32,
    anim_vel_x: f32,
    anim_vel_y: f32,
    is_animating_pointer: bool,
    accum_x: f32,
    accum_y: f32,

    last_trackpad_move_time: Instant,
    trackpad_vel_x: f32,
    trackpad_vel_y: f32,
    anim_trackpad_vel_x: f32,
    anim_trackpad_vel_y: f32,
    is_animating_trackpad: bool,
    accum_trackpad_x: f32,
    accum_trackpad_y: f32,
    last_physical_scroll_time: Instant,

    last_scroll_time: Instant,
    scroll_vel_x: f32,
    scroll_vel_y: f32,
    anim_scroll_vel_x: f32,
    anim_scroll_vel_y: f32,
    is_animating_scroll: bool,
    accum_scroll_x: f32,
    accum_scroll_y: f32,

    tap_to_click: bool,
    trackpad_disabled_by_scroll: bool,
    three_finger_start_x: Option<f32>,
    three_finger_start_y: Option<f32>,
    three_finger_gesture_triggered: bool,
}

fn send_ipc_cmd(ipc_tx: &std::sync::mpsc::Sender<crate::ipc_server::IpcRequest>, cmd: String) {
    let (reply_tx, _) = std::sync::mpsc::channel();
    let _ = ipc_tx.send(crate::ipc_server::IpcRequest { command: cmd, reply_tx });
}

fn trigger_tap_to_click_ipc(tap: bool, ipc_tx: &std::sync::mpsc::Sender<crate::ipc_server::IpcRequest>, pipe_write: libc::c_int) {
    let cmd = format!("input tap-to-click {}", tap);
    send_ipc_cmd(ipc_tx, cmd);
    unsafe {
        libc::write(pipe_write, &1u8 as *const u8 as *const libc::c_void, 1);
    }
}

fn trigger_trackpad_disabled_ipc(disabled: bool, ipc_tx: &std::sync::mpsc::Sender<crate::ipc_server::IpcRequest>, pipe_write: libc::c_int) {
    let cmd = format!("input trackpad-disabled {}", disabled);
    send_ipc_cmd(ipc_tx, cmd);
    unsafe {
        libc::write(pipe_write, &1u8 as *const u8 as *const libc::c_void, 1);
    }
}

fn trigger_expose_ipc(ipc_tx: &std::sync::mpsc::Sender<crate::ipc_server::IpcRequest>, pipe_write: libc::c_int) {
    let cmd = "expose".to_string();
    send_ipc_cmd(ipc_tx, cmd);
    unsafe {
        libc::write(pipe_write, &1u8 as *const u8 as *const libc::c_void, 1);
    }
}

fn trigger_expose_exit_ipc(ipc_tx: &std::sync::mpsc::Sender<crate::ipc_server::IpcRequest>, pipe_write: libc::c_int) {
    let cmd = "expose-exit".to_string();
    send_ipc_cmd(ipc_tx, cmd);
    unsafe {
        libc::write(pipe_write, &1u8 as *const u8 as *const libc::c_void, 1);
    }
}

fn trigger_view_next_ipc(ipc_tx: &std::sync::mpsc::Sender<crate::ipc_server::IpcRequest>, pipe_write: libc::c_int) {
    let cmd = "view-next".to_string();
    send_ipc_cmd(ipc_tx, cmd);
    unsafe {
        libc::write(pipe_write, &1u8 as *const u8 as *const libc::c_void, 1);
    }
}

fn trigger_view_prev_ipc(ipc_tx: &std::sync::mpsc::Sender<crate::ipc_server::IpcRequest>, pipe_write: libc::c_int) {
    let cmd = "view-prev".to_string();
    send_ipc_cmd(ipc_tx, cmd);
    unsafe {
        libc::write(pipe_write, &1u8 as *const u8 as *const libc::c_void, 1);
    }
}

pub fn run_input_daemon(
    mut event_queue_rx: tokio::sync::mpsc::UnboundedReceiver<InputDaemonMsg>,
    ipc_tx: std::sync::mpsc::Sender<crate::ipc_server::IpcRequest>,
    pipe_write: libc::c_int,
) -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let res: Result<(), Box<dyn std::error::Error>> = rt.block_on(async move {
        println!("[input-subsystem] Starting Clear Input Subsystem...");

        let mut uinput_file = match setup_uinput() {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[input-subsystem] FATAL: Could not initialize /dev/uinput: {}.", e);
                return Err(Box::new(e) as Box<dyn std::error::Error>);
            }
        };

        let (tx, mut rx): (UnboundedSender<CoordinatorMsg>, UnboundedReceiver<CoordinatorMsg>) = unbounded_channel();

        // Broadcast coordinate socket setup
        let (broadcast_tx, _) = tokio::sync::broadcast::channel::<String>(32);
        let socket_path = crate::paths::get_input_coords_socket_path();
        let _ = fs::remove_file(&socket_path);
        let listener = tokio::net::UnixListener::bind(&socket_path)?;
        let b_tx = broadcast_tx.clone();
        tokio::spawn(async move {
            loop {
                if let Ok((mut stream, _)) = listener.accept().await {
                    let mut rx = b_tx.subscribe();
                    tokio::spawn(async move {
                        use tokio::io::AsyncWriteExt;
                        while let Ok(msg) = rx.recv().await {
                            if stream.write_all(format!("{}\n", msg).as_bytes()).await.is_err() {
                                break;
                            }
                        }
                    });
                }
            }
        });

        // Wait for initial config
        let (initial_config, tap_to_click) = match event_queue_rx.recv().await {
            Some(InputDaemonMsg::UpdateConfig(res, tap)) => (res, tap),
            _ => {
                return Err(Box::from("Failed to receive initial input configuration") as Box<dyn std::error::Error>);
            }
        };

        // Config listener task
        let tx_config = tx.clone();
        tokio::spawn(async move {
            while let Some(msg) = event_queue_rx.recv().await {
                let _ = tx_config.send(CoordinatorMsg::DaemonMsg(msg));
            }
        });

        // Event device discovery loop
        let tx_devices = tx.clone();
        tokio::spawn(async move {
            let mut active_devices = HashSet::new();
            loop {
                if let Ok(entries) = fs::read_dir("/dev/input/by-path") {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if name.ends_with("-event-mouse") {
                                if let Ok(canonical) = fs::canonicalize(&path) {
                                    if !active_devices.contains(&canonical) {
                                        active_devices.insert(canonical.clone());
                                        let tx_c = tx_devices.clone();
                                        let device_path = canonical.clone();
                                        tokio::spawn(async move {
                                            if let Err(e) = read_device_loop(device_path.clone(), tx_c).await {
                                                eprintln!("[input-subsystem] Reader loop exited for {:?}: {}", device_path, e);
                                            }
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                sleep(Duration::from_secs(5)).await;
            }
        });

        // Central coordinator and physics ticks
        trigger_tap_to_click_ipc(tap_to_click, &ipc_tx, pipe_write);

        let mut state = PhysicsState {
            config: initial_config,
            last_move_time: Instant::now(),
            vel_x: 0.0,
            vel_y: 0.0,
            anim_vel_x: 0.0,
            anim_vel_y: 0.0,
            is_animating_pointer: false,
            accum_x: 0.0,
            accum_y: 0.0,
            last_trackpad_move_time: Instant::now(),
            trackpad_vel_x: 0.0,
            trackpad_vel_y: 0.0,
            anim_trackpad_vel_x: 0.0,
            anim_trackpad_vel_y: 0.0,
            is_animating_trackpad: false,
            accum_trackpad_x: 0.0,
            accum_trackpad_y: 0.0,
            last_physical_scroll_time: Instant::now(),
            last_scroll_time: Instant::now(),
            scroll_vel_x: 0.0,
            scroll_vel_y: 0.0,
            anim_scroll_vel_x: 0.0,
            anim_scroll_vel_y: 0.0,
            is_animating_scroll: false,
            accum_scroll_x: 0.0,
            accum_scroll_y: 0.0,
            tap_to_click,
            trackpad_disabled_by_scroll: false,
            three_finger_start_x: None,
            three_finger_start_y: None,
            three_finger_gesture_triggered: false,
        };

        let mut tick_interval = tokio::time::interval(Duration::from_millis(16)); // ~60fps

        loop {
            tokio::select! {
                Some(msg) = rx.recv() => {
                    match msg {
                        CoordinatorMsg::DaemonMsg(daemon_msg) => {
                            match daemon_msg {
                                InputDaemonMsg::UpdateConfig(cfg, tap) => {
                                    state.config = cfg;
                                    if state.tap_to_click != tap {
                                        state.tap_to_click = tap;
                                        trigger_tap_to_click_ipc(tap, &ipc_tx, pipe_write);
                                    }
                                }
                                InputDaemonMsg::SimulateMove { dx, dy } => {
                                    let _ = write_mouse_move(&mut uinput_file, dx, dy);
                                    update_pointer_coords(dx, dy);
                                }
                                InputDaemonMsg::SimulateButton { button, press } => {
                                    let val = if press { 1 } else { 0 };
                                    let _ = write_raw_event(&mut uinput_file, EV_KEY, button, val);
                                    let _ = write_raw_event(&mut uinput_file, EV_SYN, SYN_REPORT, 0);
                                }
                                InputDaemonMsg::SimulateKey { keycode, press } => {
                                    let val = if press { 1 } else { 0 };
                                    let _ = write_raw_event(&mut uinput_file, EV_KEY, keycode, val);
                                    let _ = write_raw_event(&mut uinput_file, EV_SYN, SYN_REPORT, 0);
                                }
                                InputDaemonMsg::SimulateClick { button } => {
                                    let _ = write_raw_event(&mut uinput_file, EV_KEY, button, 1);
                                    let _ = write_raw_event(&mut uinput_file, EV_SYN, SYN_REPORT, 0);
                                    let tx_clone = tx.clone();
                                    tokio::spawn(async move {
                                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                                        let _ = tx_clone.send(CoordinatorMsg::InternalReleaseButton { button });
                                    });
                                }
                                InputDaemonMsg::SimulateKeyPress { keycode } => {
                                    let _ = write_raw_event(&mut uinput_file, EV_KEY, keycode, 1);
                                    let _ = write_raw_event(&mut uinput_file, EV_SYN, SYN_REPORT, 0);
                                    let tx_clone = tx.clone();
                                    tokio::spawn(async move {
                                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                                        let _ = tx_clone.send(CoordinatorMsg::InternalReleaseKey { keycode });
                                    });
                                }
                            }
                        }
                        CoordinatorMsg::InternalReleaseButton { button } => {
                            let _ = write_raw_event(&mut uinput_file, EV_KEY, button, 0);
                            let _ = write_raw_event(&mut uinput_file, EV_SYN, SYN_REPORT, 0);
                        }
                        CoordinatorMsg::InternalReleaseKey { keycode } => {
                            let _ = write_raw_event(&mut uinput_file, EV_KEY, keycode, 0);
                            let _ = write_raw_event(&mut uinput_file, EV_SYN, SYN_REPORT, 0);
                        }
                        CoordinatorMsg::PhysicalMove { dx, dy, timestamp } => {
                            update_pointer_coords(dx, dy);
                            let dt = timestamp.duration_since(state.last_move_time).as_secs_f32();
                            state.last_move_time = timestamp;

                            state.is_animating_pointer = false;
                            state.anim_vel_x = 0.0;
                            state.anim_vel_y = 0.0;

                            if dt > 0.1 {
                                state.vel_x = 0.0;
                                state.vel_y = 0.0;
                            } else if dt > 0.001 {
                                let inst_vx = dx as f32 / dt;
                                let inst_vy = dy as f32 / dt;
                                let alpha = 0.35;
                                state.vel_x = alpha * inst_vx + (1.0 - alpha) * state.vel_x;
                                state.vel_y = alpha * inst_vy + (1.0 - alpha) * state.vel_y;
                            }
                        }
                        CoordinatorMsg::PhysicalTrackpadMove { dx, dy, timestamp } => {
                            update_pointer_coords(dx, dy);
                            let dt = timestamp.duration_since(state.last_trackpad_move_time).as_secs_f32();
                            state.last_trackpad_move_time = timestamp;

                            state.is_animating_trackpad = false;
                            state.anim_trackpad_vel_x = 0.0;
                            state.anim_trackpad_vel_y = 0.0;

                            if dt > 0.1 {
                                state.trackpad_vel_x = 0.0;
                                state.trackpad_vel_y = 0.0;
                            } else if dt > 0.001 {
                                let inst_vx = dx as f32 / dt;
                                let inst_vy = dy as f32 / dt;
                                let alpha = 0.35;
                                state.trackpad_vel_x = alpha * inst_vx + (1.0 - alpha) * state.trackpad_vel_x;
                                state.trackpad_vel_y = alpha * inst_vy + (1.0 - alpha) * state.trackpad_vel_y;
                            }
                        }
                        CoordinatorMsg::PhysicalTrackpadLift { timestamp } => {
                            if state.trackpad_disabled_by_scroll {
                                state.trackpad_disabled_by_scroll = false;
                                trigger_trackpad_disabled_ipc(false, &ipc_tx, pipe_write);
                            }
                            let time_since_scroll = timestamp.duration_since(state.last_physical_scroll_time).as_secs_f32();
                            if time_since_scroll > 0.15 {
                                if state.config.inertial_trackpad && !state.is_animating_trackpad {
                                    let time_since_last = timestamp.duration_since(state.last_trackpad_move_time).as_secs_f32();
                                    if time_since_last < 0.05 {
                                        let speed = (state.trackpad_vel_x * state.trackpad_vel_x + state.trackpad_vel_y * state.trackpad_vel_y).sqrt();
                                        if speed > 150.0 {
                                            state.is_animating_trackpad = true;
                                            state.anim_trackpad_vel_x = state.trackpad_vel_x;
                                            state.anim_trackpad_vel_y = state.trackpad_vel_y;
                                            state.accum_trackpad_x = 0.0;
                                            state.accum_trackpad_y = 0.0;
                                        }
                                    }
                                }
                            } else {
                                state.trackpad_vel_x = 0.0;
                                state.trackpad_vel_y = 0.0;
                            }
                        }
                        CoordinatorMsg::PhysicalScroll { dwx, dwy, timestamp } => {
                            if !state.trackpad_disabled_by_scroll {
                                state.trackpad_disabled_by_scroll = true;
                                trigger_trackpad_disabled_ipc(true, &ipc_tx, pipe_write);
                            }
                            let dt = timestamp.duration_since(state.last_scroll_time).as_secs_f32();
                            state.last_scroll_time = timestamp;
                            state.last_physical_scroll_time = timestamp;

                            state.is_animating_scroll = false;
                            state.anim_scroll_vel_x = 0.0;
                            state.anim_scroll_vel_y = 0.0;

                            if dt > 0.15 {
                                state.scroll_vel_x = 0.0;
                                state.scroll_vel_y = 0.0;
                            } else if dt > 0.001 {
                                let inst_vx = dwx as f32 / dt;
                                let inst_vy = dwy as f32 / dt;
                                let alpha = 0.45;
                                state.scroll_vel_x = alpha * inst_vx + (1.0 - alpha) * state.scroll_vel_x;
                                state.scroll_vel_y = alpha * inst_vy + (1.0 - alpha) * state.scroll_vel_y;
                            }
                        }
                        CoordinatorMsg::FingersReport(fingers) => {
                            if fingers.is_empty() && state.trackpad_disabled_by_scroll {
                                state.trackpad_disabled_by_scroll = false;
                                trigger_trackpad_disabled_ipc(false, &ipc_tx, pipe_write);
                            }

                            // Detect 3-finger gestures (swipe up, down, left, right)
                            if fingers.len() == 3 {
                                let avg_x = (fingers[0].x + fingers[1].x + fingers[2].x) / 3.0;
                                let avg_y = (fingers[0].y + fingers[1].y + fingers[2].y) / 3.0;
                                if let (Some(start_x), Some(start_y)) = (state.three_finger_start_x, state.three_finger_start_y) {
                                    let dy_up = start_y - avg_y; // Y decreases as fingers move up
                                    let dy_down = avg_y - start_y; // Y increases as fingers move down
                                    let dx_right = avg_x - start_x; // X increases as fingers move right
                                    let dx_left = start_x - avg_x; // X decreases as fingers move left

                                    if dy_up > 0.15 && !state.three_finger_gesture_triggered {
                                        state.three_finger_gesture_triggered = true;
                                        println!("[input-subsystem] 3-finger swipe up gesture detected. Triggering Expose mode.");
                                        trigger_expose_ipc(&ipc_tx, pipe_write);
                                    } else if dy_down > 0.15 && !state.three_finger_gesture_triggered {
                                        state.three_finger_gesture_triggered = true;
                                        println!("[input-subsystem] 3-finger swipe down gesture detected. Triggering Expose exit.");
                                        trigger_expose_exit_ipc(&ipc_tx, pipe_write);
                                    } else if dx_right > 0.15 && !state.three_finger_gesture_triggered {
                                        state.three_finger_gesture_triggered = true;
                                        println!("[input-subsystem] 3-finger swipe right gesture detected. Switching to previous tag.");
                                        trigger_view_prev_ipc(&ipc_tx, pipe_write);
                                    } else if dx_left > 0.15 && !state.three_finger_gesture_triggered {
                                        state.three_finger_gesture_triggered = true;
                                        println!("[input-subsystem] 3-finger swipe left gesture detected. Switching to next tag.");
                                        trigger_view_next_ipc(&ipc_tx, pipe_write);
                                    }
                                } else {
                                    state.three_finger_start_x = Some(avg_x);
                                    state.three_finger_start_y = Some(avg_y);
                                }
                            } else {
                                state.three_finger_start_x = None;
                                state.three_finger_start_y = None;
                                state.three_finger_gesture_triggered = false;
                            }

                            if let Ok(serialized) = serde_json::to_string(&fingers) {
                                let _ = broadcast_tx.send(serialized);
                            }
                        }
                    }
                }
                _ = tick_interval.tick() => {
                    let now = Instant::now();
                    let tick_dt = 0.016;

                    // --- Pointer physics tick ---
                    if state.config.inertial_pointer {
                        if !state.is_animating_pointer {
                            let time_since_last = now.duration_since(state.last_move_time).as_secs_f32();
                            if time_since_last >= 0.02 && time_since_last < 0.1 {
                                let speed = (state.vel_x * state.vel_x + state.vel_y * state.vel_y).sqrt();
                                if speed > 150.0 {
                                    state.is_animating_pointer = true;
                                    state.anim_vel_x = state.vel_x;
                                    state.anim_vel_y = state.vel_y;
                                    state.accum_x = 0.0;
                                    state.accum_y = 0.0;
                                }
                            }
                        } else {
                            let friction_coef = state.config.pointer_friction as f32 / 100.0;
                            state.anim_vel_x *= friction_coef;
                            state.anim_vel_y *= friction_coef;

                            let speed = (state.anim_vel_x * state.anim_vel_x + state.anim_vel_y * state.anim_vel_y).sqrt();
                            if speed < 12.0 {
                                state.is_animating_pointer = false;
                            } else {
                                state.accum_x += state.anim_vel_x * tick_dt * state.config.pointer_speed as f32;
                                state.accum_y += state.anim_vel_y * tick_dt * state.config.pointer_speed as f32;

                                let steps_x = state.accum_x.trunc() as i32;
                                let steps_y = state.accum_y.trunc() as i32;
                                state.accum_x -= steps_x as f32;
                                state.accum_y -= steps_y as f32;

                                if steps_x != 0 || steps_y != 0 {
                                    let _ = write_mouse_move(&mut uinput_file, steps_x, steps_y);
                                    update_pointer_coords(steps_x, steps_y);
                                }
                            }
                        }
                    }

                    // --- Trackpad physics tick ---
                    if state.config.inertial_trackpad && state.is_animating_trackpad {
                        let friction_coef = state.config.trackpad_friction as f32 / 100.0;
                        state.anim_trackpad_vel_x *= friction_coef;
                        state.anim_trackpad_vel_y *= friction_coef;

                        let speed = (state.anim_trackpad_vel_x * state.anim_trackpad_vel_x + state.anim_trackpad_vel_y * state.anim_trackpad_vel_y).sqrt();
                        if speed < 12.0 {
                            state.is_animating_trackpad = false;
                        } else {
                            state.accum_trackpad_x += state.anim_trackpad_vel_x * tick_dt * state.config.trackpad_speed as f32;
                            state.accum_trackpad_y += state.anim_trackpad_vel_y * tick_dt * state.config.trackpad_speed as f32;

                            let steps_x = state.accum_trackpad_x.trunc() as i32;
                            let steps_y = state.accum_trackpad_y.trunc() as i32;
                            state.accum_trackpad_x -= steps_x as f32;
                            state.accum_trackpad_y -= steps_y as f32;

                            if steps_x != 0 || steps_y != 0 {
                                let _ = write_mouse_move(&mut uinput_file, steps_x, steps_y);
                                update_pointer_coords(steps_x, steps_y);
                            }
                        }
                    }

                    // --- Scroll physics tick ---
                    if state.config.inertial_scroll {
                        if !state.is_animating_scroll {
                            let time_since_last = now.duration_since(state.last_scroll_time).as_secs_f32();
                            if time_since_last >= 0.03 && time_since_last < 0.15 {
                                let speed = (state.scroll_vel_x * state.scroll_vel_x + state.scroll_vel_y * state.scroll_vel_y).sqrt();
                                if speed > 5.0 {
                                    state.is_animating_scroll = true;
                                    state.anim_scroll_vel_x = state.scroll_vel_x;
                                    state.anim_scroll_vel_y = state.scroll_vel_y;
                                    state.accum_scroll_x = 0.0;
                                    state.accum_scroll_y = 0.0;
                                }
                            }
                        } else {
                            let friction_coef = state.config.scroll_friction as f32 / 100.0;
                            state.anim_scroll_vel_x *= friction_coef;
                            state.anim_scroll_vel_y *= friction_coef;

                            let speed = (state.anim_scroll_vel_x * state.anim_scroll_vel_x + state.anim_scroll_vel_y * state.anim_scroll_vel_y).sqrt();
                            if speed < 0.5 {
                                state.is_animating_scroll = false;
                            } else {
                                state.accum_scroll_x += state.anim_scroll_vel_x * tick_dt * state.config.scroll_speed as f32;
                                state.accum_scroll_y += state.anim_scroll_vel_y * tick_dt * state.config.scroll_speed as f32;

                                let steps_x = state.accum_scroll_x.trunc() as i32;
                                let steps_y = state.accum_scroll_y.trunc() as i32;
                                state.accum_scroll_x -= steps_x as f32;
                                state.accum_scroll_y -= steps_y as f32;

                                if steps_x != 0 || steps_y != 0 {
                                    let _ = write_scroll(&mut uinput_file, steps_x, steps_y);
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    res
}
