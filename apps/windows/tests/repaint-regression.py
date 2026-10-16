"""Reproduce input/hover flicker and scroll ghosts on the native desktop.

Run on an unlocked Windows desktop, for example:
  python apps/windows/tests/repaint-regression.py --exe target/release/ttree-windows.exe --artifacts target/repaint-regression

Starts and closes only its own TTREE process. It changes unsaved controls, never
saves settings, and verifies the original settings file remains byte-for-byte intact.
The natural screen capture is taken before any forced redraw; a static screenshot
alone cannot detect the stale pixels this regression originally produced.
"""

import os, json, time, traceback, ctypes, ctypes.wintypes as wt, hashlib, subprocess
from pathlib import Path
from PIL import ImageGrab, ImageChops
from pywinauto import Application, mouse
from pywinauto.timings import wait_until
import argparse

parser = argparse.ArgumentParser(
    description="Windows native repaint regression; requires an interactive desktop, pywinauto and Pillow."
)
parser.add_argument(
    "--exe", required=True, type=Path, help="Native TTREE executable to test"
)
parser.add_argument(
    "--artifacts",
    required=True,
    type=Path,
    help="Directory for screen samples and JSON result",
)
parser.add_argument(
    "--input-only", action="store_true", help="Only check input repaint and clear behavior"
)
parser.add_argument(
    "--window-class", default="TTreePureRustWin32MainClass",
    help="Native window class (override for an isolated test build)",
)
args = parser.parse_args()
exe = args.exe.resolve()
out = args.artifacts.resolve()
out.mkdir(exist_ok=True, parents=True)
report = {"checks": []}
proc = None
win = None
root_hwnd = None
u = ctypes.windll.user32
dwm = ctypes.windll.dwmapi
u.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
u.SendMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
u.SendMessageW.restype = ctypes.c_ssize_t
u.SetWinEventHook.restype = wt.HANDLE
u.UnhookWinEvent.argtypes = [wt.HANDLE]


class RECT(ctypes.Structure):
    _fields_ = [
        ("left", wt.LONG),
        ("top", wt.LONG),
        ("right", wt.LONG),
        ("bottom", wt.LONG),
    ]


class SCROLLINFO(ctypes.Structure):
    _fields_ = [
        ("size", wt.UINT),
        ("mask", wt.UINT),
        ("min", ctypes.c_int),
        ("max", ctypes.c_int),
        ("page", wt.UINT),
        ("pos", ctypes.c_int),
        ("track", ctypes.c_int),
    ]


class SCROLLBARINFO(ctypes.Structure):
    _fields_ = [
        ("size", wt.DWORD),
        ("rect", RECT),
        ("line", ctypes.c_int),
        ("top", ctypes.c_int),
        ("bottom", ctypes.c_int),
        ("reserved", ctypes.c_int),
        ("states", wt.DWORD * 6),
    ]


profile = Path(os.environ["APPDATA"]) / "com.samwstu.ttree/settings.json"
before = profile.read_bytes() if profile.exists() else None
try:
    proc = subprocess.Popen(
        [str(exe), "--hidden"],
        stdout=(out / "stdout.log").open("w"),
        stderr=(out / "stderr.log").open("w"),
    )
    app = Application(backend="win32").connect(process=proc.pid, timeout=15)
    win = app.window(class_name=args.window_class, visible_only=False)
    win.wait("exists", timeout=10)
    root_hwnd = win.handle
    for _ in range(6):
        assert not u.IsWindowVisible(win.handle)
        time.sleep(0.05)
    report["checks"].append("startup_layout_keeps_hidden_window_hidden")
    u.SendMessageW(win.handle, 0x111, 901, 0)
    win.wait("visible enabled ready", timeout=10)
    u.SendMessageW(win.handle, 0x111, 105, 0)
    win.set_focus()

    def c(i):
        return win.child_window(control_id=i, visible_only=False)

    def shot():
        dwm.DwmFlush()
        r = win.rectangle()
        return ImageGrab.grab(bbox=(r.left, r.top, r.right, r.bottom))

    # Observe actual Win32 geometry notifications, including intermediate sizes
    # that a screenshot taken after SendMessage returns would miss.
    source = c(201)
    source.set_focus()
    source.set_edit_text("输入")
    geometry_events = []
    event_callback_type = ctypes.WINFUNCTYPE(
        None, wt.HANDLE, wt.DWORD, wt.HWND, wt.LONG, wt.LONG, wt.DWORD, wt.DWORD
    )

    @event_callback_type
    def on_geometry_event(hook, event, hwnd, object_id, child_id, thread, stamp):
        if hwnd == source.handle and object_id == 0:
            geometry_events.append(event)

    u.SetWinEventHook.argtypes = [
        wt.DWORD, wt.DWORD, wt.HMODULE, event_callback_type,
        wt.DWORD, wt.DWORD, wt.DWORD,
    ]
    hook = u.SetWinEventHook(
        0x800B, 0x800B, None, on_geometry_event, proc.pid, 0, 0
    )
    assert hook, "could not observe native source input geometry"

    def pump_events():
        message = wt.MSG()
        while u.PeekMessageW(ctypes.byref(message), None, 0, 0, 1):
            u.TranslateMessage(ctypes.byref(message))
            u.DispatchMessageW(ctypes.byref(message))

    try:
        time.sleep(0.05)
        pump_events()
        geometry_events.clear()
        bounds = source.rectangle()
        first_frame = shot()
        for char in "连续输入时窗口应该保持稳定 Native input stays stable 1234567890":
            u.SendMessageW(source.handle, 0x102, ord(char), 0)
            pump_events()
            assert source.rectangle() == bounds
        time.sleep(0.05)
        pump_events()
        report["typing_geometry_changes"] = len(geometry_events)
        assert not geometry_events, (
            f"typing resized the source input {len(geometry_events)} times"
        )
        geometry_events.clear()
        # Exercise the native edit replacement path used by paste/selection.
        for text in ["粘贴的内容", "replacement text", "最后一段输入"]:
            u.SendMessageW(source.handle, 0xB1, 0, -1)  # EM_SETSEL
            replacement = ctypes.create_unicode_buffer(text)
            u.SendMessageW(source.handle, 0xC2, 1, ctypes.addressof(replacement))
            pump_events()
            assert source.window_text() == text and source.rectangle() == bounds
        time.sleep(0.05)
        pump_events()
        # Native edit replacements can notify client/scrollbar location changes
        # while retaining the same outer bounds (checked for every replacement).
        report["paste_native_location_notifications"] = len(geometry_events)
        first_frame.save(out / "input-first.png")
        shot().save(out / "input-last.png")
    finally:
        u.UnhookWinEvent(hook)
    report["checks"].append("continuous_input_and_paste_keep_native_geometry_stable")
    source.set_edit_text("多行内容\r\n" * 30)
    assert u.GetWindowLongW(source.handle, -16) & 0x200000
    source.set_edit_text("短文本")
    assert not u.GetWindowLongW(source.handle, -16) & 0x200000
    assert source.rectangle() == bounds
    report["checks"].append("input_scrollbar_tracks_text_overflow_without_window_layout")
    assert c(206).is_visible()
    source.set_edit_text("")
    assert not c(206).is_visible()
    assert source.rectangle().width() > bounds.width()
    source.set_edit_text("恢复输入")
    assert c(206).is_visible() and source.rectangle() == bounds
    clear_bounds = c(206).rectangle()
    u.SendMessageW(root_hwnd, 0x111, 206, 0)
    assert source.window_text() == "" and not c(206).is_visible()
    report["checks"].append("clear_action_and_empty_input_width_follow_text_state")
    # The exposed strip contains the old clear button but no blinking caret.
    # Check naturally presented pixels before requesting a reference redraw.
    natural = shot()
    u.RedrawWindow(root_hwnd, None, None, 0x585)
    reference = shot()
    window_bounds = win.rectangle()
    exposed = (
        bounds.right - window_bounds.left,
        bounds.top - window_bounds.top,
        clear_bounds.right - window_bounds.left,
        bounds.bottom - window_bounds.top,
    )
    changed = sum(
        1 for pixel in ImageChops.difference(
            natural.crop(exposed), reference.crop(exposed)
        ).getdata() if max(pixel[:3]) > 8
    )
    report["cleared_input_stale_pixels"] = changed
    natural.save(out / "input-cleared.png")
    assert changed == 0, f"clearing input left {changed} stale pixels"
    report["checks"].append("clearing_input_repaints_exposed_strip_without_ghosts")

    if args.input_only:
        report["profile_unchanged"] = (
            profile.read_bytes() if profile.exists() else None
        ) == before
        assert report["profile_unchanged"]
        report["ok"] = True
        raise SystemExit(0)

    hover = {}
    for ident in [104, 105, 103, 507]:
        r = c(ident).rectangle()
        u.SetCursorPos(r.left + 8, r.top + 8)
        time.sleep(0.08)
        frames = []
        for i in range(60):
            u.SetCursorPos(r.left + 8 + i % 12, r.top + 8 + i % 5)
            f = ImageGrab.grab(bbox=(r.left, r.top, r.right, r.bottom))
            frames.append(f)
        hashes = {hashlib.sha256(f.tobytes()).hexdigest() for f in frames}
        hover[str(ident)] = len(hashes)
        frames[-1].save(out / f"hover-{ident}.png")
        assert len(hashes) == 1, (
            f"button {ident} changed while pointer stayed inside: {len(hashes)} frames"
        )
    report["hover_distinct_frames"] = hover
    report["checks"].append("240_hover_frames_remain_stable_inside_each_button")
    c(103).click_input()
    c(503).click_input()
    win.move_window(width=480, height=440)
    for i, ident in enumerate([417, 418, 419, 420, 421]):
        c(ident).set_edit_text("Ctrl+Shift+" + str(i + 1))
    view = c(508)
    vr = view.rectangle()
    u.SetCursorPos(vr.right - 28, vr.top + 35)
    time.sleep(0.1)

    def scrollinfo():
        s = SCROLLINFO()
        s.size = ctypes.sizeof(s)
        s.mask = 0x17
        assert u.GetScrollInfo(view.handle, 1, ctypes.byref(s))
        return s

    def canonical_difference():
        natural = shot()
        u.RedrawWindow(win.handle, None, None, 0x585)
        reference = shot()
        diff = ImageChops.difference(natural, reference)
        return sum(1 for p in diff.getdata() if max(p[:3]) > 8), natural, reference

    diffs = []
    frames = []
    for cycle in range(8):
        for i in range(12):
            delta = -120 if cycle % 2 == 0 else 120
            u.SendMessageW(view.handle, 0x20A, (delta & 0xFFFF) << 16, 0)
            if i in [0, 5, 11]:
                frames.append(shot())
        changed, natural, reference = canonical_difference()
        diffs.append(changed)
        if changed:
            natural.save(out / f"cycle-{cycle}-natural.png")
            reference.save(out / f"cycle-{cycle}-reference.png")
    assert max(diffs) == 0, f"scroll left stale pixels: {diffs}"
    report["scroll_pixels_changed_by_forced_redraw"] = diffs
    report["checks"].append("96_wheel_steps_leave_no_stale_pixels")
    # A wheel event over a normal field must move the settings viewport, retaining its value.
    u.SendMessageW(view.handle, 0x115, 6, 0)
    start = scrollinfo().pos
    field = c(417)
    text = field.window_text()
    u.SendMessageW(field.handle, 0x20A, (0xFF88 << 16), 0)
    assert scrollinfo().pos > start and field.window_text() == text
    report["checks"].append("wheel_over_field_scrolls_page_without_changing_value")
    # Drag the actual native scrollbar thumb, exercising WM_VSCROLL/SIF_TRACKPOS.
    u.SendMessageW(view.handle, 0x115, 6, 0)
    bar = SCROLLBARINFO()
    bar.size = ctypes.sizeof(bar)
    assert u.GetScrollBarInfo(view.handle, ctypes.c_long(-5), ctypes.byref(bar))
    x = (bar.rect.left + bar.rect.right) // 2
    y = bar.rect.top + (bar.top + bar.bottom) // 2
    target = min(bar.rect.bottom - bar.line - 4, y + 90)
    mouse.press(coords=(x, y))
    for i in range(1, 11):
        u.SetCursorPos(x, y + (target - y) * i // 10)
        time.sleep(0.01)
    mouse.release(coords=(x, target))
    assert scrollinfo().pos > 0
    changed, natural, reference = canonical_difference()
    report["thumb_drag_stale_pixels"] = changed
    natural.save(out / "thumb-drag.png")
    assert changed == 0
    report["checks"].append("native_thumb_drag_keeps_scene_and_fields_aligned")
    # Moving between pages with and without scrollbars must not retain old client widths.
    for _ in range(5):
        c(504).click_input()
        assert c(423).is_visible()
        c(503).click_input()
        assert c(417).is_visible()
    changed, natural, reference = canonical_difference()
    report["section_switch_stale_pixels"] = changed
    assert changed == 0
    report["checks"].append(
        "section_switch_and_scrollbar_visibility_do_not_leave_ghosts"
    )
    # Resize through narrow/wide row breakpoints while scrolled.
    for width in [380, 640, 500, 480]:
        win.move_window(width=width, height=440)
        u.SendMessageW(view.handle, 0x115, 7, 0)
        changed, _, _ = canonical_difference()
        assert changed == 0
    report["checks"].append("responsive_resize_while_scrolled_remains_aligned")
    # Exercise a real dropdown list; buffered owner draw must preserve popup item coordinates.
    c(504).click_input()
    combo = c(423)
    combo.select(2)
    u.SendMessageW(win.handle, 0x111, (1 << 16) | 423, combo.handle)
    assert combo.selected_index() == 2
    report["checks"].append("native_dropdown_selection_still_works")
    u.SendMessageW(view.handle, 0x115, 6, 0)
    shot().save(out / "settings.png")
    c(505).click_input()
    u.SendMessageW(win.handle, 0x111, 507, 0)
    assert not u.IsWindowVisible(win.handle)
    # Hidden relayout triggered by a settings command must not display the app unexpectedly.
    u.SendMessageW(win.handle, 0x111, 904, 0)
    win.wait("visible", timeout=10)
    c(504).click_input()
    shot().save(out / "final.png")
    report["checks"].append("hide_and_reopen_still_work_after_batched_layout")
    report["profile_unchanged"] = (
        profile.read_bytes() if profile.exists() else None
    ) == before
    assert report["profile_unchanged"]
    report["ok"] = True
except Exception:
    report["ok"] = False
    report["error"] = traceback.format_exc()
    if win:
        try:
            shot().save(out / "failure.png")
        except Exception:
            pass
finally:
    if proc and proc.poll() is None:
        if root_hwnd:
            u.PostMessageW(root_hwnd, 0x111, 905, 0)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.terminate()
            proc.wait(timeout=5)
    report["owned_process_exited"] = proc is None or proc.poll() is not None
    (out / "report.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8"
    )

raise SystemExit(0 if report.get("ok") else 1)
