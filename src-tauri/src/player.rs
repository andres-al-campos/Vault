//! The in-app player: libmpv drawing inside Vault's own window.
//!
//! The window's web view is transparent, and a plain view sits underneath it
//! with a layer mpv renders into. The page leaves a hole where the video goes
//! and draws its tabs and controls around it, so everything the user touches
//! stays HTML. mpv does the decoding, which is what makes AV1 play on a Mac
//! whose own player can't.
//!
//! mpv is not handed a view to attach to (its `wid` option): on macOS that
//! made it open a window of its own. It draws through its render API instead,
//! into the framebuffer Core Animation gives the layer.
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::atomic::{AtomicPtr, Ordering};

use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{class, msg_send, sel};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use serde::Serialize;

extern "C" {
    fn mpv_create() -> *mut c_void;
    fn mpv_set_option_string(h: *mut c_void, name: *const c_char, value: *const c_char) -> c_int;
    fn mpv_initialize(h: *mut c_void) -> c_int;
    fn mpv_command(h: *mut c_void, args: *const *const c_char) -> c_int;
    fn mpv_get_property_string(h: *mut c_void, name: *const c_char) -> *mut c_char;
    fn mpv_free(p: *mut c_void);
    fn mpv_error_string(e: c_int) -> *const c_char;
    fn mpv_render_context_create(out: *mut *mut c_void, mpv: *mut c_void, params: *const RenderParam) -> c_int;
    fn mpv_render_context_render(ctx: *mut c_void, params: *const RenderParam) -> c_int;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

// An opaque Core Graphics colour space, named so it can be passed to AppKit.
#[repr(C)]
struct CGColorSpace {
    _private: [u8; 0],
}
unsafe impl objc2::RefEncode for CGColorSpace {
    const ENCODING_REF: objc2::Encoding = objc2::Encoding::Pointer(&objc2::Encoding::Struct("CGColorSpace", &[]));
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    static kCGColorSpaceSRGB: *const c_void;
    fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut CGColorSpace;
}

#[link(name = "QuartzCore", kind = "framework")]
#[link(name = "OpenGL", kind = "framework")]
extern "C" {
    fn glGetIntegerv(name: u32, out: *mut c_int);
    fn glFlush();
    fn CGLChoosePixelFormat(attrs: *const c_int, pix: *mut *mut c_void, npix: *mut c_int) -> c_int;
}

// mpv's render API, as far as this file uses it (render.h, render_gl.h).
const PARAM_END: c_int = 0;
const PARAM_API_TYPE: c_int = 1;
const PARAM_OPENGL_INIT: c_int = 2;
const PARAM_OPENGL_FBO: c_int = 3;
const PARAM_FLIP_Y: c_int = 4;
const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
const GL_VIEWPORT: u32 = 0x0BA2;

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}
#[repr(C)]
struct GlInit {
    get_proc_address: extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    ctx: *mut c_void,
}
#[repr(C)]
struct GlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

static MPV: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static RENDER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static VIEW: AtomicPtr<AnyObject> = AtomicPtr::new(std::ptr::null_mut());
static LAYER: AtomicPtr<AnyObject> = AtomicPtr::new(std::ptr::null_mut());

fn mpv_err(code: c_int) -> String {
    unsafe { CStr::from_ptr(mpv_error_string(code)).to_string_lossy().into_owned() }
}

fn handle() -> Result<*mut c_void, String> {
    let h = MPV.load(Ordering::SeqCst);
    if h.is_null() {
        Err("The player didn't start. Check mpv is installed (`brew install mpv`), then reopen Vault.".into())
    } else {
        Ok(h)
    }
}

extern "C" fn gl_symbol(_: *mut c_void, name: *const c_char) -> *mut c_void {
    // RTLD_DEFAULT on macOS: the OpenGL framework is already loaded.
    unsafe { dlsym(-2isize as *mut c_void, name) }
}

// Core Animation calls this with the layer's GL context current. mpv draws its
// current frame into whatever framebuffer the layer has bound. The render
// context is made here, on the first draw, because this is the one place that
// context is guaranteed to be current.
extern "C-unwind" fn draw(this: &AnyObject, _: Sel, _cgl: *mut c_void, _pf: *mut c_void, _t: f64, _ts: *const c_void) {
    unsafe {
        let mut render = RENDER.load(Ordering::SeqCst);
        if render.is_null() {
            let mpv = MPV.load(Ordering::SeqCst);
            if mpv.is_null() {
                return;
            }
            let mut init = GlInit { get_proc_address: gl_symbol, ctx: std::ptr::null_mut() };
            let params = [
                RenderParam { kind: PARAM_API_TYPE, data: c"opengl".as_ptr() as *mut c_void },
                RenderParam { kind: PARAM_OPENGL_INIT, data: &mut init as *mut GlInit as *mut c_void },
                RenderParam { kind: PARAM_END, data: std::ptr::null_mut() },
            ];
            let rc = mpv_render_context_create(&mut render, mpv, params.as_ptr());
            if rc < 0 {
                eprintln!("mpv render context: {}", mpv_err(rc));
                return;
            }
            RENDER.store(render, Ordering::SeqCst);
        }
        let mut fbo: c_int = 0;
        glGetIntegerv(GL_FRAMEBUFFER_BINDING, &mut fbo);
        // mpv fits the picture to the size it's told. That comes from the
        // layer itself; the GL viewport is the fallback, since it can still
        // be the old size on the first draws after the layer is resized,
        // which stretched the picture going into fullscreen.
        let bounds: NSRect = msg_send![this, bounds];
        let scale: f64 = msg_send![this, contentsScale];
        let (mut w, mut h) = ((bounds.size.width * scale).round() as c_int, (bounds.size.height * scale).round() as c_int);
        if w <= 0 || h <= 0 {
            let mut viewport = [0 as c_int; 4];
            glGetIntegerv(GL_VIEWPORT, viewport.as_mut_ptr());
            (w, h) = (viewport[2], viewport[3]);
        }
        let mut target = GlFbo { fbo, w, h, internal_format: 0 };
        let mut flip: c_int = 1;
        let params = [
            RenderParam { kind: PARAM_OPENGL_FBO, data: &mut target as *mut GlFbo as *mut c_void },
            RenderParam { kind: PARAM_FLIP_Y, data: &mut flip as *mut c_int as *mut c_void },
            RenderParam { kind: PARAM_END, data: std::ptr::null_mut() },
        ];
        mpv_render_context_render(render, params.as_ptr());
        glFlush();
    }
}

// A 3.2 core context; the layer's default is the legacy 2.1 one.
extern "C-unwind" fn pixel_format(_this: &AnyObject, _: Sel, _mask: u32) -> *mut c_void {
    // kCGLPFAOpenGLProfile = 3.2 core, kCGLPFAAccelerated, kCGLPFADoubleBuffer
    let attrs: [c_int; 6] = [99, 0x3200, 73, 5, 0, 0];
    let (mut pix, mut n) = (std::ptr::null_mut(), 0);
    unsafe { CGLChoosePixelFormat(attrs.as_ptr(), &mut pix, &mut n) };
    pix
}

fn layer_class() -> &'static AnyClass {
    let mut b = ClassBuilder::new(c"VaultPlayerLayer", class!(CAOpenGLLayer)).expect("player layer class registered twice");
    unsafe {
        b.add_method(
            sel!(drawInCGLContext:pixelFormat:forLayerTime:displayTime:),
            draw as extern "C-unwind" fn(_, _, _, _, _, _),
        );
        b.add_method(sel!(copyCGLPixelFormatForDisplayMask:), pixel_format as extern "C-unwind" fn(_, _, _) -> _);
    }
    b.register()
}

/// Puts the video view under the window's web view and starts mpv. Called once,
/// on the main thread, when the app sets up. A failure leaves the rest of
/// Vault working; the player commands then say what's wrong.
pub fn init(window: &tauri::WebviewWindow) -> Result<(), String> {
    unsafe {
        let ns_window = window.ns_window().map_err(|e| e.to_string())? as *mut AnyObject;
        let content: *mut AnyObject = msg_send![ns_window, contentView];
        let view: *mut AnyObject = msg_send![class!(NSView), alloc];
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(16.0, 9.0));
        let view: *mut AnyObject = msg_send![view, initWithFrame: frame];
        let layer: *mut AnyObject = msg_send![layer_class(), new];
        let scale: f64 = msg_send![ns_window, backingScaleFactor];
        let _: () = msg_send![layer, setContentsScale: scale];
        // Without a colour space the layer's pixels go to the display as they
        // are, and on a wide-gamut Mac screen the video comes out more
        // saturated than the same frame anywhere else, the page's own still
        // of it included. mpv draws sRGB, so say so.
        let srgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        if !srgb.is_null() {
            let _: () = msg_send![layer, setColorspace: srgb];
        }
        let _: () = msg_send![view, setLayer: layer];
        let _: () = msg_send![view, setWantsLayer: true];
        let _: () = msg_send![view, setHidden: true];
        // Tauri's transparent window is clear all the way through. Only the
        // web view needs to be: with a black window behind, a stage the video
        // view hasn't caught up with yet (a resize, going fullscreen) shows
        // black instead of the desktop.
        let black: *mut AnyObject = msg_send![class!(NSColor), blackColor];
        let _: () = msg_send![ns_window, setBackgroundColor: black];
        // NSWindowBelow: under the web view, which is transparent where the
        // page leaves its hole.
        let nil: *mut AnyObject = std::ptr::null_mut();
        let _: () = msg_send![content, addSubview: view, positioned: -1isize, relativeTo: nil];
        VIEW.store(view, Ordering::SeqCst);
        LAYER.store(layer, Ordering::SeqCst);

        let h = mpv_create();
        if h.is_null() {
            return Err("mpv could not be created.".into());
        }
        for (k, v) in [
            ("vo", "libmpv"),
            ("hwdec", "auto-safe"),
            // A finished video stays on its last frame instead of unloading,
            // so the page can offer to replay it.
            ("keep-open", "yes"),
            // The keyboard's play/pause key, headphone buttons and Control
            // Center. mpv ships this for macOS but leaves it off as a library;
            // it arrives as a key press, which the default bindings turn into
            // a pause toggle. No other key reaches mpv: it has no window.
            ("input-media-keys", "yes"),
            ("input-default-bindings", "yes"),
        ] {
            let (k, v) = (CString::new(k).unwrap(), CString::new(v).unwrap());
            let rc = mpv_set_option_string(h, k.as_ptr(), v.as_ptr());
            if rc < 0 {
                eprintln!("mpv option {k:?}={v:?}: {}", mpv_err(rc));
            }
        }
        let rc = mpv_initialize(h);
        if rc < 0 {
            return Err(format!("mpv did not start: {}", mpv_err(rc)));
        }
        MPV.store(h, Ordering::SeqCst);
    }
    Ok(())
}

fn command(args: &[&str]) -> Result<(), String> {
    let h = handle()?;
    let owned: Vec<CString> = args.iter().map(|a| CString::new(*a).unwrap_or_default()).collect();
    let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    let rc = unsafe { mpv_command(h, ptrs.as_ptr()) };
    if rc < 0 {
        Err(format!("mpv refused {:?}: {}", args, mpv_err(rc)))
    } else {
        Ok(())
    }
}

fn property(name: &str) -> Option<String> {
    let h = MPV.load(Ordering::SeqCst);
    if h.is_null() {
        return None;
    }
    let name = CString::new(name).ok()?;
    unsafe {
        let p = mpv_get_property_string(h, name.as_ptr());
        if p.is_null() {
            return None;
        }
        let s = CStr::from_ptr(p).to_string_lossy().into_owned();
        mpv_free(p as *mut c_void);
        Some(s)
    }
}

/// Loads a file at `start` seconds in, playing or held on that frame.
#[tauri::command(async)]
pub fn player_load(path: String, start: f64, paused: bool) -> Result<(), String> {
    if !std::path::Path::new(&path).is_file() {
        return Err(format!(
            "The file isn't there: {path}. If it's on an external drive, connect it; otherwise press Sync and download it again."
        ));
    }
    // Set before the file loads, so a held video never sounds for a moment.
    command(&["set", "pause", if paused { "yes" } else { "no" }])?;
    command(&["loadfile", &path, "replace", "-1", &format!("start={start}")])
}

/// What the page asks of a playing video.
#[tauri::command(async)]
pub fn player_set(action: String, value: Option<f64>) -> Result<(), String> {
    let v = value.unwrap_or(0.0).to_string();
    match action.as_str() {
        "play" => command(&["set", "pause", "no"]),
        "pause" => command(&["set", "pause", "yes"]),
        "seek" => command(&["seek", &v, "absolute"]),
        "volume" => command(&["set", "volume", &v]),
        "mute" => command(&["cycle", "mute"]),
        "stop" => command(&["stop"]),
        other => Err(format!("The player has no \"{other}\" action. It takes play, pause, seek, volume, mute and stop.")),
    }
}

#[derive(Serialize, Default)]
pub struct PlayerState {
    position: f64,
    duration: f64,
    paused: bool,
    muted: bool,
    /// The picture for the current position is up: the file is open, the
    /// seek to where it starts has landed, and the output has a frame. The
    /// length alone is known well before that.
    ready: bool,
    /// The video reached its end and is sitting on the last frame.
    ended: bool,
    /// The file these numbers are about, so a report that crosses a tab
    /// switch can't be taken for the new video's. None while nothing is loaded.
    path: Option<String>,
}

#[tauri::command(async)]
pub fn player_state() -> PlayerState {
    let num = |name: &str| property(name).and_then(|s| s.parse::<f64>().ok());
    let yes = |name: &str| property(name).as_deref() == Some("yes");
    PlayerState {
        position: num("time-pos").unwrap_or(0.0),
        duration: num("duration").unwrap_or(0.0),
        paused: yes("pause"),
        muted: yes("mute"),
        ready: num("time-pos").is_some() && !yes("seeking") && yes("vo-configured"),
        ended: yes("eof-reached"),
        path: property("path"),
    }
}

/// Where the page's hole is, in web coordinates (top-left origin), or hidden
/// when the player isn't on screen. AppKit counts from the bottom-left.
#[tauri::command(async)]
pub fn player_place(window: tauri::WebviewWindow, visible: bool, x: f64, top: f64, w: f64, h: f64, win_h: f64) {
    let _ = window.run_on_main_thread(move || unsafe {
        let (view, layer) = (VIEW.load(Ordering::SeqCst), LAYER.load(Ordering::SeqCst));
        if view.is_null() {
            return;
        }
        let _: () = msg_send![view, setHidden: !visible];
        // Redrawn every display refresh while shown, and not at all while
        // hidden.
        let _: () = msg_send![layer, setAsynchronous: visible];
        if visible {
            // Core Animation eases a layer to its new size over a quarter of
            // a second unless told not to, and the picture wobbles with it.
            let _: () = msg_send![class!(CATransaction), begin];
            let _: () = msg_send![class!(CATransaction), setDisableActions: true];
            let frame = NSRect::new(NSPoint::new(x, win_h - top - h), NSSize::new(w, h));
            let _: () = msg_send![view, setFrame: frame];
            // The layer follows its view, but say so outright and ask for a
            // fresh drawable at the new size.
            let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h));
            let _: () = msg_send![layer, setBounds: bounds];
            let _: () = msg_send![layer, setNeedsDisplay];
            let _: () = msg_send![class!(CATransaction), commit];
        }
    });
}

/// The player's fullscreen is the window's own. With `on` it switches the
/// window; without, it answers whether the window is fullscreen now.
#[tauri::command(async)]
pub fn player_fullscreen(window: tauri::WebviewWindow, on: Option<bool>) -> Result<bool, String> {
    match on {
        Some(on) => {
            window.set_fullscreen(on).map_err(|e| {
                format!("Couldn't switch fullscreen: {e}. Use the green button in the window's title bar instead.")
            })?;
            // The move to or from fullscreen takes the keyboard away from the
            // web view, and the page's keys (F, Esc, Space) stop arriving.
            // Hand it back now and keep handing it back through the animation
            // (well under a second), so the keys work the moment it ends
            // instead of at some fixed time after.
            // (The window's own set_focus only raises the window; it's the
            // web view inside that has to be given the keys.)
            let focus = move || {
                let webview: &tauri::Webview = window.as_ref();
                let _ = webview.set_focus();
            };
            focus();
            std::thread::spawn(move || {
                for _ in 0..15 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    focus();
                }
            });
            Ok(on)
        }
        None => window
            .is_fullscreen()
            .map_err(|e| format!("Couldn't tell whether the window is fullscreen: {e}. Reopen Vault if the player's controls look wrong.")),
    }
}

/// Saves the frame on screen to a file and answers its path. The page shows
/// it over the stage while this video loads again, so switching tabs goes from
/// picture to picture instead of through black.
#[tauri::command(async)]
pub fn player_still(id: String) -> Result<String, String> {
    // The id becomes a file name.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(format!("\"{id}\" isn't a video id, so no picture was saved for it."));
    }
    let dir = std::env::temp_dir().join("vault-stills");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Couldn't create {}: {e}. Check the disk has space.", dir.display()))?;
    // A new name each time, so the page never shows a cached older frame.
    let prefix = format!("{id}-");
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for old in entries.flatten() {
            if old.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(old.path());
            }
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("{prefix}{stamp}.jpg"));
    let path = path.to_string_lossy().into_owned();
    command(&["screenshot-to-file", &path, "video"])?;
    Ok(path)
}
