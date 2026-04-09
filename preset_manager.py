import ctypes
import ctypes.wintypes as wt
import json
import logging
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import psutil

logging.basicConfig(level=logging.INFO, format="[%(asctime)s] %(message)s")
log = logging.getLogger()

BASE    = Path("C:/monitor")
PRESETS = BASE / "presets.json"
FFMPEG  = BASE / "ffmpeg/bin/ffmpeg.exe"
RTSP    = "rtsp://localhost:8554/display"
FPS     = 30

VIRTUAL_DISPLAY_ID          = "Root\\MttVDD"
DISPLAY_DEVICE_ATTACHED_TO_DESKTOP = 0x00000001
ENUM_CURRENT_SETTINGS       = 0xFFFFFFFF

# Sentinel values used in presets.json to select the capture source.
INPUT_DXGI   = ["__dxgi__"]
INPUT_IMAGE  = ["__image__"]
INPUT_WINDOW = ["__window__"]


# ---------------------------------------------------------------------------
# Windows structs
# ---------------------------------------------------------------------------

class DISPLAY_DEVICE(ctypes.Structure):
    _fields_ = [
        ("cb",           wt.DWORD),
        ("DeviceName",   ctypes.c_wchar * 32),
        ("DeviceString", ctypes.c_wchar * 128),
        ("StateFlags",   wt.DWORD),
        ("DeviceID",     ctypes.c_wchar * 128),
        ("DeviceKey",    ctypes.c_wchar * 128),
    ]


class DEVMODEW(ctypes.Structure):
    # Full DEVMODEW layout (220 bytes) — display-device union variant assumed.
    # Union at offset 76 maps dmPosition POINTL directly (printer fields ignored).
    _fields_ = [
        ("dmDeviceName",         ctypes.c_wchar * 32),   # 0
        ("dmSpecVersion",        ctypes.c_ushort),        # 64
        ("dmDriverVersion",      ctypes.c_ushort),        # 66
        ("dmSize",               ctypes.c_ushort),        # 68
        ("dmDriverExtra",        ctypes.c_ushort),        # 70
        ("dmFields",             ctypes.c_uint32),        # 72
        ("dmPositionX",          ctypes.c_int32),         # 76  — POINTL.x
        ("dmPositionY",          ctypes.c_int32),         # 80  — POINTL.y
        ("dmDisplayOrientation", ctypes.c_uint32),        # 84
        ("dmDisplayFixedOutput", ctypes.c_uint32),        # 88
        ("dmColor",              ctypes.c_short),         # 92
        ("dmDuplex",             ctypes.c_short),         # 94
        ("dmYResolution",        ctypes.c_short),         # 96
        ("dmTTOption",           ctypes.c_short),         # 98
        ("dmCollate",            ctypes.c_short),         # 100
        ("dmFormName",           ctypes.c_wchar * 32),   # 102
        ("dmLogPixels",          ctypes.c_ushort),        # 166
        ("dmBitsPerPel",         ctypes.c_uint32),        # 168
        ("dmPelsWidth",          ctypes.c_uint32),        # 172
        ("dmPelsHeight",         ctypes.c_uint32),        # 176
        ("dmDisplayFlags",       ctypes.c_uint32),        # 180
        ("dmDisplayFrequency",   ctypes.c_uint32),        # 184
        ("dmICMMethod",          ctypes.c_uint32),        # 188
        ("dmICMIntent",          ctypes.c_uint32),        # 192
        ("dmMediaType",          ctypes.c_uint32),        # 196
        ("dmDitherType",         ctypes.c_uint32),        # 200
        ("dmReserved1",          ctypes.c_uint32),        # 204
        ("dmReserved2",          ctypes.c_uint32),        # 208
        ("dmPanningWidth",       ctypes.c_uint32),        # 212
        ("dmPanningHeight",      ctypes.c_uint32),        # 216
    ]                                                     # total: 220 bytes


# ---------------------------------------------------------------------------
# Runtime monitor registry
# ---------------------------------------------------------------------------

class MonitorInfo:
    """Single active display adapter. Keyed by stable DeviceName (e.g. ``\\\\.\\DISPLAY2``)."""
    def __init__(self, key: str, index: int, x: int, y: int, w: int, h: int, is_virtual: bool):
        self.key        = key         # DeviceName — stable across index shifts
        self.index      = index       # active-adapter enumeration / DXGI output index
        self.x          = x
        self.y          = y
        self.w          = w
        self.h          = h
        self.is_virtual = is_virtual

    def __str__(self):
        kind = "virtual" if self.is_virtual else "physical"
        return (f"{self.key} ({kind})  pos=({self.x},{self.y})"
                f"  size={self.w}x{self.h}  idx={self.index}")


class MonitorRegistry:
    """
    Live monitor list keyed by stable DeviceName (``\\\\.\\DISPLAY1``, etc.).
    Built at startup and refreshed on demand, independent of presets.json.
    Presets resolve their capture target through this registry at switch time
    so that stale positional indexes in JSON cannot direct capture to the
    wrong display after a topology change.
    """

    def __init__(self):
        self._monitors: dict[str, MonitorInfo] = {}
        self._lock = threading.Lock()

    def refresh(self) -> dict[str, MonitorInfo]:
        """Re-enumerate all displays attached to the desktop via Win32 API.

        DXGI output indices are resolved by name via _dxgi_output_map() so that
        monitors sharing the same resolution are never confused.  Falls back to
        the EnumDisplayDevices enumeration order when DXGI mapping is unavailable.
        """
        dxgi_map = _dxgi_output_map()   # {DeviceName: ddagrab output_idx}

        user32 = ctypes.windll.user32
        found: dict[str, MonitorInfo] = {}
        enum_index = 0   # fallback counter when DXGI map is empty
        i = 0
        while True:
            adapter = DISPLAY_DEVICE()
            adapter.cb = ctypes.sizeof(adapter)
            if not user32.EnumDisplayDevicesW(None, i, ctypes.byref(adapter), 0):
                break
            active = bool(adapter.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP)
            if active:
                dm = DEVMODEW()
                dm.dmSize = ctypes.sizeof(DEVMODEW)
                if user32.EnumDisplaySettingsW(
                    adapter.DeviceName, ENUM_CURRENT_SETTINGS, ctypes.byref(dm)
                ):
                    is_virt = VIRTUAL_DISPLAY_ID.lower() in adapter.DeviceID.lower()
                    # Prefer the DXGI-derived index (exact name match); fall back
                    # to enumeration order only when DXGI initialisation failed.
                    dxgi_idx = dxgi_map.get(adapter.DeviceName, enum_index)
                    found[adapter.DeviceName] = MonitorInfo(
                        key=adapter.DeviceName,
                        index=dxgi_idx,
                        x=dm.dmPositionX,
                        y=dm.dmPositionY,
                        w=dm.dmPelsWidth,
                        h=dm.dmPelsHeight,
                        is_virtual=is_virt,
                    )
                    if is_virt:
                        log.info("virtual display identified: %s (DeviceID=%s)",
                                 adapter.DeviceName, adapter.DeviceID)
                enum_index += 1
            i += 1

        with self._lock:
            self._monitors = found
        log.info("monitor registry refreshed: %d display(s) — %s",
                 len(found), ", ".join(found.keys()))
        return found

    def get_all(self) -> dict[str, MonitorInfo]:
        with self._lock:
            return dict(self._monitors)

    def get_virtual(self) -> "MonitorInfo | None":
        """Return the first IDD virtual display, or None if not present."""
        with self._lock:
            return next((m for m in self._monitors.values() if m.is_virtual), None)


# ---------------------------------------------------------------------------
# DXGI output map  — DeviceName → ddagrab output_idx
# ---------------------------------------------------------------------------

class _DXGI_OUTPUT_DESC(ctypes.Structure):
    """Matches the Windows SDK DXGI_OUTPUT_DESC layout exactly."""
    _fields_ = [
        ("DeviceName",         ctypes.c_wchar * 32),
        ("DesktopCoordinates", wt.RECT),
        ("AttachedToDesktop",  wt.BOOL),
        ("Rotation",           ctypes.c_uint),
        ("Monitor",            wt.HANDLE),
    ]


class _GUID(ctypes.Structure):
    _fields_ = [
        ("Data1", ctypes.c_uint32),
        ("Data2", ctypes.c_uint16),
        ("Data3", ctypes.c_uint16),
        ("Data4", ctypes.c_uint8 * 8),
    ]


# IID_IDXGIFactory = {7b7166ec-21c7-44ae-b21a-c9ae321ae369}
_IID_IDXGIFactory = _GUID(
    0x7b7166ec, 0x21c7, 0x44ae,
    (ctypes.c_uint8 * 8)(0xb2, 0x1a, 0xc9, 0xae, 0x32, 0x1a, 0xe3, 0x69),
)

_DXGI_ERROR_NOT_FOUND = 0x887A0002

# COM vtable call signatures — defined once at module level so _dxgi_output_map
# doesn't reallocate them on every adapter/output iteration.
_COM_ENUM_FN    = ctypes.CFUNCTYPE(ctypes.c_long, ctypes.c_void_p,
                                    ctypes.c_uint, ctypes.POINTER(ctypes.c_void_p))
_COM_GETDESC_FN = ctypes.CFUNCTYPE(ctypes.c_long, ctypes.c_void_p,
                                    ctypes.POINTER(_DXGI_OUTPUT_DESC))
_COM_RELEASE_FN = ctypes.CFUNCTYPE(ctypes.c_ulong, ctypes.c_void_p)


def _dxgi_output_map() -> dict[str, int]:
    """
    Walk IDXGIFactory → IDXGIAdapter → IDXGIOutput and read each output's
    DeviceName from DXGI_OUTPUT_DESC.  Returns {DeviceName: global_output_idx}
    where global_output_idx is the value ddagrab's output_idx= parameter expects.

    No resolution matching — each display is identified by its exact name so
    monitors that share a resolution (e.g. two 1920×1080 displays) are always
    resolved to the correct DXGI output.  Returns {} on any failure so callers
    can fall back to the EnumDisplayDevices enumeration order.
    """
    def _vtable(obj: ctypes.c_void_p) -> ctypes.Array:
        vptr = ctypes.cast(obj, ctypes.POINTER(ctypes.c_void_p)).contents
        return ctypes.cast(vptr, ctypes.POINTER(ctypes.c_void_p))

    def _release(obj: ctypes.c_void_p, vtable: ctypes.Array) -> None:
        _COM_RELEASE_FN(vtable[2])(obj)

    try:
        dxgi_dll = ctypes.WinDLL("dxgi")
        _CreateDXGIFactory = dxgi_dll.CreateDXGIFactory
        _CreateDXGIFactory.restype  = ctypes.c_long
        _CreateDXGIFactory.argtypes = [ctypes.POINTER(_GUID),
                                       ctypes.POINTER(ctypes.c_void_p)]
        factory = ctypes.c_void_p()
        hr = _CreateDXGIFactory(ctypes.byref(_IID_IDXGIFactory),
                                ctypes.byref(factory))
        if hr != 0 or not factory:
            log.warning("_dxgi_output_map: CreateDXGIFactory hr=0x%08x", hr & 0xFFFFFFFF)
            return {}
    except Exception as exc:
        log.warning("_dxgi_output_map: init error: %s", exc)
        return {}

    result: dict[str, int] = {}
    global_idx = 0
    vt_factory = _vtable(factory)

    try:
        # vtable slot 7 → IDXGIFactory::EnumAdapters(UINT, IDXGIAdapter**)
        EnumAdapters = _COM_ENUM_FN(vt_factory[7])

        adapter_i = 0
        while True:
            adapter = ctypes.c_void_p()
            hr = EnumAdapters(factory, adapter_i, ctypes.byref(adapter))
            if hr & 0xFFFFFFFF == _DXGI_ERROR_NOT_FOUND or not adapter:
                break
            if hr != 0:
                break
            vt_adapter = _vtable(adapter)
            try:
                # vtable slot 7 → IDXGIAdapter::EnumOutputs(UINT, IDXGIOutput**)
                EnumOutputs = _COM_ENUM_FN(vt_adapter[7])

                output_i = 0
                while True:
                    output = ctypes.c_void_p()
                    hr2 = EnumOutputs(adapter, output_i, ctypes.byref(output))
                    if hr2 & 0xFFFFFFFF == _DXGI_ERROR_NOT_FOUND or not output:
                        break
                    if hr2 != 0:
                        break
                    vt_output = _vtable(output)
                    try:
                        # vtable slot 7 → IDXGIOutput::GetDesc(DXGI_OUTPUT_DESC*)
                        GetDesc = _COM_GETDESC_FN(vt_output[7])
                        desc = _DXGI_OUTPUT_DESC()
                        if GetDesc(output, ctypes.byref(desc)) == 0:
                            result[desc.DeviceName] = global_idx
                        global_idx += 1
                    finally:
                        _release(output, vt_output)
                    output_i += 1
            finally:
                _release(adapter, vt_adapter)
            adapter_i += 1

    except Exception as exc:
        log.warning("_dxgi_output_map: enumeration error: %s", exc)
    finally:
        _release(factory, vt_factory)

    log.info("_dxgi_output_map: %s",
             ", ".join(f"{k}→{v}" for k, v in result.items()) or "(empty)")
    return result


# ---------------------------------------------------------------------------
# FFmpeg capability probe
# ---------------------------------------------------------------------------

def probe_ddagrab() -> bool:
    try:
        r = subprocess.run(
            [str(FFMPEG), "-filters"],
            capture_output=True, text=True, timeout=15,
        )
        return "ddagrab" in r.stdout
    except Exception:
        return False


# ---------------------------------------------------------------------------
# FFmpeg input builders
# ---------------------------------------------------------------------------

def input_ddagrab(vmon: MonitorInfo) -> list:
    # ddagrab outputs a D3D GPU texture — hwdownload brings it to CPU memory,
    # then format=bgra→yuv420p for the software encoder.
    return [
        "-f", "lavfi",
        "-i", f"ddagrab=output_idx={vmon.index}:framerate={FPS}:draw_mouse=0",
        "-vf", "hwdownload,format=bgra,format=yuv420p",
    ]


def input_gdigrab(vmon: MonitorInfo) -> list:
    return [
        "-f", "gdigrab",
        "-framerate", str(FPS),
        "-offset_x", str(vmon.x),
        "-offset_y", str(vmon.y),
        "-video_size", f"{vmon.w}x{vmon.h}",
        "-i", "desktop",
    ]


def input_window(title: str) -> list:
    # Captures a single window by its exact title on any display.
    # gdigrab self-throttles to the requested framerate; no -re needed.
    # The scale filter floors dimensions to an even number so libx264
    # never rejects the frame with "width/height not divisible by 2".
    return [
        "-f", "gdigrab",
        "-framerate", str(FPS),
        "-i", f"title={title}",
        "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2",
    ]


def encode_flags(out_fps: int = FPS, use_amf: bool = False) -> list:
    # One keyframe per second so HLS 1s segments always contain a sync point.
    gop = max(2, out_fps)
    if use_amf:
        codec_flags = [
            "-vcodec", "h264_amf",
            "-profile:v", "constrained_baseline",  # WebRTC requires Baseline/CB
            "-level", "3.1",                        # WebRTC max; AMF defaults to 4.2
            "-rc", "cbr",
            "-b:v", "6M",
            "-bufsize", "500k",
            "-usage", "ultralowlatency",
            "-quality", "speed",
            "-header_spacing", str(gop),  # SPS/PPS in every IDR frame
        ]
    else:
        codec_flags = [
            "-vcodec", "libx264",
            "-preset", "ultrafast",
            "-tune", "zerolatency",
            "-b:v", "4M",
        ]
    return codec_flags + [
        "-pix_fmt", "yuv420p",
        "-g", str(gop), "-keyint_min", str(gop),
        "-fflags", "nobuffer",
        "-f", "rtsp", "-rtsp_transport", "tcp", RTSP,
    ]


def ensure_standby(vmon: MonitorInfo) -> Path:
    standby = BASE / "standby.png"
    if not standby.exists():
        log.info("generating standby.png (%dx%d black frame)...", vmon.w, vmon.h)
        subprocess.run(
            [
                str(FFMPEG), "-y",
                "-f", "lavfi",
                "-i", f"color=black:s={vmon.w}x{vmon.h}",
                "-frames:v", "1",
                str(standby),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=True,
        )
    return standby


# ---------------------------------------------------------------------------
# Process management
# ---------------------------------------------------------------------------

def kill_proc(proc):
    if proc is None:
        return
    try:
        parent = psutil.Process(proc.pid)
        for child in parent.children(recursive=True):
            child.terminate()
        parent.terminate()
    except psutil.NoSuchProcess:
        pass
    except psutil.AccessDenied:
        log.warning("access denied terminating pid %d", proc.pid)
    finally:
        try:
            proc.wait(timeout=5)
        except Exception:
            proc.kill()


# ---------------------------------------------------------------------------
# Global state
# ---------------------------------------------------------------------------

_state = {
    "ffmpeg":           None,
    "ff_log_fh":        None,   # open file handle for ffmpeg stderr; closed on next switch
    "app":              None,
    "preset":           None,
    "monitor_key":      "",     # monitor override in effect for current preset
    "lock":             threading.Lock(),
    "switching":        False,  # guard against concurrent /switch calls
    "vmon":             None,   # MonitorInfo for virtual display, refreshed on each switch
    "monitor_registry": None,   # MonitorRegistry, set at startup
    "ddagrab":          False,  # capability flag, set at startup
    "amf":              False,  # capability flag, set at startup
}


def load_presets() -> dict:
    with open(PRESETS) as f:
        return json.load(f)


def resolve_input(raw_input: list, vmon: MonitorInfo, ddagrab: bool,
                  window_title: str = "") -> list:
    if raw_input == INPUT_DXGI:
        # Live capture — hardware already throttles to FPS, no -re needed.
        return input_ddagrab(vmon) if ddagrab else input_gdigrab(vmon)
    if raw_input == INPUT_IMAGE:
        standby = ensure_standby(vmon)
        # -re: read at native rate (2 fps) so timestamps stay in sync with wall clock.
        return ["-re", "-stream_loop", "-1", "-f", "image2", "-framerate", "2", "-i", str(standby)]
    if raw_input == INPUT_WINDOW:
        if not window_title:
            raise ValueError("preset uses __window__ but has no window_title field")
        return input_window(window_title)
    # lavfi / other synthetic sources also need -re for real-time pacing.
    return ["-re"] + raw_input


MAX_FFMPEG_RETRIES = 3

def _ffmpeg_watchdog(proc: subprocess.Popen, preset: str, log_path: Path,
                     started_at: float, retry: int = 0):
    """
    Runs in a background thread. On unexpected exit, waits 3s and restarts
    the preset so FFmpeg reconnects (e.g. after MediaMTX restarts).
    Retries are capped at MAX_FFMPEG_RETRIES; resets to 0 after a 30s stable run.
    """
    rc = proc.wait()
    with _state["lock"]:
        still_current = _state["ffmpeg"] is proc
    if not still_current:
        return  # switch() already replaced this process

    if rc in (0, -1, -15):
        log.info("ffmpeg (%s) stopped normally (rc=%d)", preset, rc)
        return

    ran_for = time.time() - started_at
    next_retry = 0 if ran_for > 30 else retry + 1

    if next_retry > MAX_FFMPEG_RETRIES:
        log.error("ffmpeg (%s) failed %d times in a row (rc=%d) — check %s",
                  preset, MAX_FFMPEG_RETRIES, rc, log_path)
        if preset != "image":
            log.warning("falling back to 'image' preset")
            switch("image")
        return

    log.warning("ffmpeg (%s) exited (rc=%d, ran %.1fs) — restarting in 3s [%d/%d]",
                preset, rc, ran_for, next_retry, MAX_FFMPEG_RETRIES)
    time.sleep(3)
    switch(preset, _retry=next_retry, monitor_key=_state["monitor_key"])


def switch(name: str, _retry: int = 0, monitor_key: str = "") -> tuple[bool, str]:
    """
    Switch to a named preset.  If *monitor_key* is provided (a DeviceName from
    GET /monitors, e.g. ``\\\\.\\DISPLAY2``) that specific display is used as the
    capture source for __dxgi__ / __image__ presets, overriding the default
    virtual-monitor auto-detect.
    """
    presets = load_presets()
    if name not in presets:
        return False, f"unknown preset: {name}"

    p = presets[name]
    ddagrab: bool = _state["ddagrab"]
    use_amf: bool = _state["amf"]

    # Re-discover monitors at switch time so a topology change (driver restart,
    # display re-enumeration) cannot cause FFmpeg to capture the wrong output.
    raw = p["ffmpeg_input"]
    if raw in (INPUT_DXGI, INPUT_IMAGE):
        registry: MonitorRegistry = _state["monitor_registry"]
        registry.refresh()
        if monitor_key:
            all_monitors = registry.get_all()
            if monitor_key not in all_monitors:
                return False, f"unknown monitor key: {monitor_key!r} (see GET /monitors)"
            vmon = all_monitors[monitor_key]
        else:
            vmon = registry.get_virtual()
            if vmon is None:
                return False, "virtual display not found — is the driver active?"
        _state["vmon"] = vmon
        log.info("monitor resolved at switch time: %s", vmon)
    else:
        vmon = _state["vmon"]

    with _state["lock"]:
        if _state["switching"] and _retry == 0:
            return False, "switch already in progress"
        _state["switching"] = True

    try:
        # Kill old processes and drain the cooldown outside the lock so /status
        # remains responsive during teardown and app startup_delay.
        log.info("switching to '%s'", name)
        old_fh = None
        with _state["lock"]:
            old_name  = _state["preset"]
            old_proc  = _state["ffmpeg"]; _state["ffmpeg"] = None
            old_fh    = _state["ff_log_fh"]; _state["ff_log_fh"] = None
            old_app   = _state["app"];   _state["app"]   = None

        # Run teardown command for the outgoing preset before killing processes.
        if old_name and old_name in presets:
            old_p = presets[old_name]
            if old_p.get("teardown_app"):
                td_cmd = [old_p["teardown_app"]] + old_p.get("teardown_app_args", [])
                log.info("teardown '%s': %s", old_name, " ".join(td_cmd))
                subprocess.Popen(
                    td_cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
                )
                time.sleep(old_p.get("teardown_delay", 1))

        kill_proc(old_proc)
        kill_proc(old_app)
        if old_fh is not None:
            try:
                old_fh.close()
            except Exception:
                pass
        time.sleep(0.4)

        app_proc = None
        if p.get("app"):
            cmd = [p["app"]] + p.get("app_args", [])
            log.info("launching app: %s", " ".join(cmd))
            app_proc = subprocess.Popen(
                cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
            )
            time.sleep(p.get("startup_delay", 2))

        if raw == INPUT_DXGI or raw == INPUT_WINDOW:
            out_fps = FPS
        elif raw == INPUT_IMAGE:
            out_fps = 2
        else:
            out_fps = 1
        inp = resolve_input(raw, vmon, ddagrab, p.get("window_title", ""))
        ffcmd = [str(FFMPEG), "-y"] + inp + encode_flags(out_fps, use_amf)
        log.info("ffmpeg cmd: %s", " ".join(ffcmd))

        ff_log = BASE / "logs" / f"ffmpeg_{name}.log"
        ff_log_fh = open(ff_log, "w", encoding="utf-8", errors="replace")
        proc = subprocess.Popen(
            ffcmd,
            stdout=subprocess.DEVNULL,
            stderr=ff_log_fh,
        )

        with _state["lock"]:
            _state["ffmpeg"]      = proc
            _state["ff_log_fh"]   = ff_log_fh
            _state["app"]         = app_proc
            _state["preset"]      = name
            _state["monitor_key"] = monitor_key
        log.info("'%s' active (ffmpeg pid %d) — stderr → %s", name, proc.pid, ff_log)

    finally:
        with _state["lock"]:
            _state["switching"] = False

    threading.Thread(
        target=_ffmpeg_watchdog,
        args=(proc, name, ff_log, time.time(), _retry),
        daemon=True,
    ).start()

    return True, name


# ---------------------------------------------------------------------------
# HTTP control API  — http://localhost:9090
# ---------------------------------------------------------------------------

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        parsed = urlparse(self.path)
        qs = parse_qs(parsed.query)

        if parsed.path == "/switch":
            name = qs.get("preset", [None])[0]
            if not name:
                # ?preset= omitted — reuse the active preset so that
                # ?monitor= alone is enough to re-point capture.
                name = _state["preset"]
            if not name:
                self._reply(400, "missing ?preset= (no active preset to reuse)")
                return
            presets = load_presets()
            if name not in presets:
                self._reply(400, f"unknown preset: {name}")
                return
            monitor_key = qs.get("monitor", [""])[0]
            if monitor_key:
                known = _state["monitor_registry"].get_all()
                if monitor_key not in known:
                    keys = ", ".join(known.keys()) or "(none)"
                    self._reply(400, f"unknown monitor key: {monitor_key!r} — available: {keys}")
                    return
            threading.Thread(
                target=switch, args=(name,), kwargs={"monitor_key": monitor_key},
                daemon=True,
            ).start()
            msg = f"switching to '{name}'"
            if monitor_key:
                msg += f" on monitor {monitor_key!r}"
            self._reply(202, msg)

        elif parsed.path == "/status":
            self._reply(200, _state["preset"] or "none")

        elif parsed.path == "/presets":
            self._reply(200, ", ".join(load_presets().keys()))

        elif parsed.path == "/monitors":
            registry: MonitorRegistry = _state["monitor_registry"]
            registry.refresh()
            data = [
                {
                    "key":        m.key,
                    "index":      m.index,
                    "resolution": f"{m.w}x{m.h}",
                    "is_virtual": m.is_virtual,
                }
                for m in registry.get_all().values()
            ]
            self._reply_json(200, data)

        else:
            self._reply(404, "not found")

    def _write(self, code: int, body: bytes, ctype: str) -> None:
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _reply(self, code: int, body: str) -> None:
        self._write(code, body.encode(), "text/plain")

    def _reply_json(self, code: int, data) -> None:
        self._write(code, json.dumps(data).encode(), "application/json")

    def log_message(self, *_):
        pass


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    # Kill any FFmpeg processes left over from a previous run so they don't
    # compete for the same RTSP publish slot (overridePublisher race).
    _killed = 0
    for _p in psutil.process_iter(["name"]):
        try:
            if "ffmpeg" in _p.info["name"].lower():
                _p.terminate()
                _killed += 1
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass
    if _killed:
        log.info("killed %d stale ffmpeg process(es) from previous run", _killed)
        time.sleep(0.5)

    log.info("building monitor registry (DeviceID filter=%s)...", VIRTUAL_DISPLAY_ID)
    registry = MonitorRegistry()
    registry.refresh()
    _state["monitor_registry"] = registry

    vmon = registry.get_virtual()
    if vmon is None:
        raise RuntimeError(
            "Virtual display not found — is the virtual-display-rs driver active?"
        )
    log.info("virtual monitor: %s", vmon)
    _state["vmon"] = vmon

    ddagrab = probe_ddagrab()
    _state["ddagrab"] = ddagrab
    capture_method = "ddagrab" if ddagrab else "gdigrab"
    log.info("capture method: %s (output_idx=%d)", capture_method, vmon.index)

    _state["amf"] = False
    log.info("encoder: libx264 (h264_amf disabled until WebRTC level issue resolved)")

    # Create runtime directories if absent
    (BASE / "logs").mkdir(exist_ok=True)
    (BASE / "media").mkdir(exist_ok=True)
    log.info("logs → %s", BASE / "logs")

    switch("image")

    log.info("control API listening on :9090")
    ThreadingHTTPServer(("0.0.0.0", 9090), Handler).serve_forever()
