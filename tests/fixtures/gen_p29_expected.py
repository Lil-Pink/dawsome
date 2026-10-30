"""Known answers for plugin windows. A VST3 plugin's own editor
(`IPlugView`) in a window of ours, the frame we give it (`IPlugFrame`, with its run loop on
Linux), the XEmbed messages our window sends the editor's, the X11 requests every plugin
window opens with and resizes by, and CLAP's test interface in a window of ours (the "clap" key,
its rules below).

Expected values come from the rules and the test plugin's declared behavior restated here, never
from running Dawsome. The test plugin is our own code (crates/test-plugin-vst3); its sixth class,
`DawsomeTestView1` "Dawsome Test View", sounds as its sine does (gen_p27_expected.py's formula,
whose helpers are copied in below: `python3 -I` cannot import a sibling file) and has the sine's
parameters, link and MIDI mapping, with a controller of its own, `DawsomeTestCtrl2`, whose
`createView("editor")` makes an editor (any other name: none). The editor logs each call the
host makes into it; on Linux it is an XEmbed client.

The rules:
  open      the host asks the controller for its editor, then whether it takes our window type
            (X11EmbedWindowID on Linux), then for the window (Linux: an X connection; none: the
            editor is let go and the reason is "Plugin windows need X11 (XWayland)"), the
            editor's size (640x480 when it will not say) and whether it resizes, opens a window
            of ours of that size, gives the editor its frame, attaches it to our window and asks
            its size once more. The editor answers 400x300 and resizes unless it is told it is
            fixed; given a frame that answers IRunLoop it makes its own window, a child of ours
            of its size with _XEMBED_INFO [0, 1] (version 0, MAPPED), which it does not map, and
            registers two file descriptors with the frame's run loop (its X connection, a
            pipe's read end); a frame without one is refused (DPF's rule).
  xembed    our window hears its children (SUBSTRUCTURE_NOTIFY) and its focus (FOCUS_CHANGE)
            besides itself (STRUCTURE_NOTIFY); a child that appears in it is watched
            (PROPERTY_CHANGE | STRUCTURE_NOTIFY) and its _XEMBED_INFO read; with one it is
            embedded: _XEMBED [time 0, EMBEDDED_NOTIFY 0, 0, our window, version 0], mapped while
            its MAPPED flag is set, unmapped when it is cleared, and when our window has the
            focus WINDOW_ACTIVATE then FOCUS_IN (CURRENT). Our window's FocusIn (detail neither
            Inferior nor Pointer) sends every embedded child WINDOW_ACTIVATE then FOCUS_IN; its
            FocusOut FOCUS_OUT then WINDOW_DEACTIVATE. A child is never sized by us.
  resize    a plugin's resizeView(view, rect): off the main thread false; a null or another
            view invalid; while closing or inside its own resizeView false; the size held to 1
            to 32767 a side equal to the one kept true and nothing else; else our window takes
            it (a fixed one's least and most size too), it is kept, the editor's onSize is
            called with {0, 0, w, h}, true. A user's resize of our window (a ConfigureNotify of
            our window's own; the last of a tick's for one window): equal to the size kept, or a
            fixed editor, nothing; else the editor's checkSizeConstraint, which holds it to
            200..1600 x 150..1200, gives the size kept, our window takes it when it differs,
            and onSize; a size held to the one kept puts our window back alone (no onSize: the
            editor has it); our window's own echo of a size is then nothing. A window manager
            may hold our window bigger than asked (KWin keeps a managed window at least 2x48, so
            0x0's 1x1 is not given): what it gives comes back as a user's resize.
  close     the window's close button (WM_DELETE_WINDOW), releasing the plugin, or the host
            going: the editor is removed, its frame taken away (setFrame null) and let go ("view
            freed"), our window closed; the plugin's parts then terminate, the controller first,
            as the host orders them, unless its audio half is still alive (it then keeps them).
  pins      every plugin window (CLAP's too) is made black with STRUCTURE_NOTIFY (1 << 17)
            alone besides; a VST3 one adds SUBSTRUCTURE_NOTIFY (1 << 19) and FOCUS_CHANGE
            (1 << 21); WM_CLASS is "<id>\\0<id>\\0" for the application id the app sets. Every
            resize of a plugin window, both formats', is one ConfigureWindow request (opcode 12,
            5 words: the window, value-mask width | height (4 | 8), the two sides held to
            1..32767), little-endian as this machine sends it.
  clap      CLAP's test interface (crates/test-plugin, `org.dawsome.test.gui` in the file
            dawsome_test_gui.clap): the host asks whether it takes X11 embedded, creates it, asks
            its size (400x300) and whether it resizes (yes, unless it is told it is fixed), opens
            our window of that size (a fixed one with its least and most size that size), gives
            it our window as its parent and shows it; the interface makes its own window, a
            child of ours of its size, and maps it. Its adjust_size holds a size to 200..1600 x
            150..1200, then to the largest 4:3 size inside that (Surge XT's interface holds its
            aspect too). A plugin's request_resize is answered true and the interface takes the
            size at once; our window takes it unless it is the size kept (a fixed one's least
            and most size too), and is kept; no adjust_size or set_size. A user's resize (our
            window's own ConfigureNotify, the last of a tick's): equal to the size kept, or a
            fixed interface, nothing; else adjust_size; a size held to the one kept puts our
            window back alone (no set_size: the interface has it); else it is kept, set_size,
            and our window takes it when it is not the size the window manager gave. What our
            window is asked for there is asked again while the window manager has not reported
            that size, after 250 ms quiet, 40 times at most per size it reported (the settling
            VST3's frame runs). Closing: destroy, then our window.

Run:
  python3 -I gen_p29_expected.py              print the known answers
  python3 -I gen_p29_expected.py --write DIR  also write the files below into DIR
  python3 -I gen_p29_expected.py --check DIR  exit 1 unless the files in DIR are what --write
                                              writes

Files:
  p29_vst3_view.json   p27_vst3_sine with the View class and a gain-0.5 preset of its class
  p29_expected.json    every expected value the tests read
"""
import base64, hashlib, json, math, os, struct, sys


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def t2s(tick, sr, mbpm, ppq=96):
    # the project's one tick->sample rule, exact integer math
    return tick * 60 * sr * 1000 // (mbpm * ppq)


def J(d):
    return json.dumps(d, separators=(",", ":"))


def class_id(name):
    raw = name.encode("ascii")
    assert len(raw) == 16, name
    return raw.hex().upper()


VIEW = class_id("DawsomeTestView1")
CTRL2 = class_id("DawsomeTestCtrl2")
NAME = "Dawsome Test View"
assert VIEW == "446177736F6D65546573745669657731" and CTRL2 == "446177736F6D65546573744374726C32"


# --- the fixture (gen_p27's helpers and layout) ----------------------------------------------
def component_state(gain):
    return b"OTP1" + struct.pack("<d", gain)


CONTROLLER_STATE = b"OTC1" + struct.pack("<I", 0)


def preset(cid, comp, cont):
    header_size = 4 + 4 + 32 + 8
    data, entries, offset = b"", [], header_size
    for tag, bytes_ in [(b"Comp", comp), (b"Cont", cont)]:
        entries.append(tag + struct.pack("<qq", offset, len(bytes_)))
        data += bytes_
        offset += len(bytes_)
    header = b"VST3" + struct.pack("<i", 1) + cid.encode("ascii") + struct.pack("<q", offset)
    return header + data + b"List" + struct.pack("<i", len(entries)) + b"".join(entries)


def b64(raw):
    return base64.b64encode(raw).decode()


def project(sr, channels, notes, next_channel_id):
    return {"format_version": 2, "sample_rate": sr, "tempo_mbpm": 120000, "ppq": 96,
            "next_channel_id": next_channel_id, "channels": channels,
            "patterns": [{"id": 1, "name": "Pattern 1", "length_bars": 1, "notes": notes}]}


def plugin_channel(cid, name, pid, state):
    return {"id": cid, "name": name, "kind": "plugin",
            "processor": {"id": pid, "name": name, "state": state},
            "volume": 1.0, "pan": 0.0, "mute": False}


VIEW_HALF = preset(VIEW, component_state(0.5), CONTROLLER_STATE)
assert len(VIEW_HALF) == 116  # gen_p27's sine preset at 0.5, the class id the View's
FIX = {"p29_vst3_view": project(48000, [plugin_channel(1, NAME, "vst3:" + VIEW, b64(VIEW_HALF))],
                                [{"channel": 1, "tick": 24, "length": 96, "velocity": 1.0, "key": 69}], 2)}


# --- its sound: gen_p27's sine ---------------------------------------------------------------
def sine(n, gain, vel=1.0, key=69, sr=48000):
    f = 440.0 * 2.0 ** ((key - 69) / 12.0)
    return f32(gain * vel * math.cos(math.tau * f * n / sr))


def sha256_of(frames):
    sha = hashlib.sha256()
    for value in frames:
        sha.update(struct.pack("<ff", value, value))
    return sha.hexdigest()


def render_view():
    sr = 48000
    out = [0.0] * t2s(384, sr, 120000)
    s0, s1 = t2s(24, sr, 120000), t2s(120, sr, 120000)
    for k in range(s0, s1):
        out[k] = sine(k - s0, 0.5)
    return out


RENDER = render_view()
S0, S1 = t2s(24, 48000, 120000), t2s(120, 48000, 120000)
assert (S0, S1) == (6000, 30000)
# The spec's probes (vst3-host-phase3.md, W1), to ten digits.
assert ["%.10g" % RENDER[k] for k in (S0, S0 + 1, S1)] == ["0.5", "0.4991708994", "0"]
# The same frames as gen_p27's sine render (p27_expected.json's sha256 for p27_vst3_sine).
assert sha256_of(RENDER) == "676f9c7fd3da21008248656b4ef9628e2da5ec61ec9779055be183d61ab8abe1"


# --- the editor's log ------------------------------------------------------------------------
SIZE = (400, 300)
LEAST, MOST = (200, 150), (1600, 1200)
NESTED = (500, 400)


def side(pixels):
    """A side both window systems take: 1 to 32767."""
    return min(max(pixels, 1), 32767)


def size_text(size):
    return "%dx%d" % size


def open_log(fixed=False):
    return ["createView editor", "isPlatformTypeSupported X11EmbedWindowID", "getSize " + size_text(SIZE),
            "canResize " + ("no" if fixed else "yes"), "setFrame frame runloop", "attached X11EmbedWindowID",
            "getSize " + size_text(SIZE)]


class Frame:
    """S6's resizeView and S7's user resize over the editor's declared answers."""

    def __init__(self, fixed=False):
        self.kept, self.window, self.child, self.fixed = SIZE, SIZE, SIZE, fixed
        self.inside, self.nested, self.log = False, False, []

    def on_size(self, size):
        self.log.append("onSize " + size_text(size))
        self.child = size  # the editor sizes its own window
        if self.nested:
            self.nested = False
            asked = NESTED
            self.log.append("resizeView %s -> %s" % (size_text(asked), self.resize_view(asked)))

    def resize_view(self, asked):
        if self.inside:
            return "false"
        size = (side(asked[0]), side(asked[1]))
        if size == self.kept:
            return "true"
        self.inside = True
        self.window, self.kept = size, size
        self.on_size(size)
        self.inside = False
        return "true"

    def plugin_resize(self, asked):
        """The editor's own `resizeView` of `asked` (the test's export), as it logs it."""
        result = self.resize_view(asked)
        self.log.append("resizeView %s -> %s" % (size_text(asked), result))

    def user_resize(self, size):
        if self.fixed or size == self.kept:
            return
        held = (min(max(size[0], LEAST[0]), MOST[0]), min(max(size[1], LEAST[1]), MOST[1]))
        self.log.append("checkSizeConstraint %s -> %s" % (size_text(size), size_text(held)))
        if held == self.kept:
            self.window = held
            return
        self.kept = held
        self.window = held
        self.on_size(held)
        # Our window's echo of the size it took is equal to the one kept: nothing more.
        self.user_resize(self.window)


def plugin_resizes():
    frame = Frame()
    steps = []
    for asked, nested in [((300, 200), False), ((300, 200), False), ((320, 240), True), ((0, 0), False)]:
        frame.log = []
        frame.nested = nested
        frame.plugin_resize(asked)
        steps.append({"asked": list(asked), "nested": nested, "log": frame.log, "window": list(frame.window),
                      "child": list(frame.child)})
    fixed = Frame(fixed=True)
    fixed.plugin_resize((300, 200))
    return steps, {"open_log": open_log(fixed=True), "hints_before": list(SIZE), "asked": [300, 200],
                   "log": fixed.log, "window": list(fixed.window), "hints_after": list(fixed.window)}


RESIZES, FIXED_RESIZE = plugin_resizes()
# By hand (the spec's W4): a first resize sizes both windows and calls onSize; the same size again
# is answered true with no onSize; inside onSize a second resizeView is refused; 0x0 is 1x1.
assert [s["log"] for s in RESIZES] == [
    ["onSize 300x200", "resizeView 300x200 -> true"], ["resizeView 300x200 -> true"],
    ["onSize 320x240", "resizeView 500x400 -> false", "resizeView 320x240 -> true"],
    ["onSize 1x1", "resizeView 0x0 -> true"]]
assert RESIZES[3]["window"] == [1, 1] and FIXED_RESIZE["log"] == ["onSize 300x200", "resizeView 300x200 -> true"]


def user_resizes():
    """The resizable editor's cases in turn on one window (each from the size the one before left),
    then the fixed editor's."""
    cases = []
    frames = {False: Frame(), True: Frame(fixed=True)}
    for fixed, size in [(False, (100, 100)), (False, (150, 100)), (False, (2000, 2000)), (True, (500, 500))]:
        frame = frames[fixed]
        frame.log = []
        frame.user_resize(size)
        cases.append({"fixed": fixed, "size": list(size), "log": frame.log, "window": list(frame.window)})
    return cases


USER_RESIZES = user_resizes()
assert [c["log"] for c in USER_RESIZES] == [
    ["checkSizeConstraint 100x100 -> 200x150", "onSize 200x150"],
    ["checkSizeConstraint 150x100 -> 200x150"],
    ["checkSizeConstraint 2000x2000 -> 1600x1200", "onSize 1600x1200"], []]
assert USER_RESIZES[1]["window"] == [200, 150] and USER_RESIZES[3]["window"] == list(SIZE)


# --- XEmbed and the X11 pins -----------------------------------------------------------------
EMBEDDED_NOTIFY, WINDOW_ACTIVATE, WINDOW_DEACTIVATE, FOCUS_IN, FOCUS_OUT = 0, 1, 2, 4, 5
FOCUS_CURRENT = 0
XEMBED_MAPPED = 1
NOTIFY_INFERIOR, NOTIFY_NONLINEAR, NOTIFY_POINTER = 2, 3, 5
STRUCTURE_NOTIFY, SUBSTRUCTURE_NOTIFY, FOCUS_CHANGE, PROPERTY_CHANGE = 1 << 17, 1 << 19, 1 << 21, 1 << 22


def xembed(opcode, detail=0, data1=0, data2=0):
    """An _XEMBED message's five words: time (CurrentTime), opcode, detail, data1, data2."""
    return [0, opcode, detail, data1, data2]


OURS = 0x0123_4567  # a window id of ours, standing in
XEMBED = {
    "embedded_notify": xembed(EMBEDDED_NOTIFY, 0, OURS, 0), "our_window": OURS,
    "window_activate": xembed(WINDOW_ACTIVATE), "window_deactivate": xembed(WINDOW_DEACTIVATE),
    "focus_in": xembed(FOCUS_IN, FOCUS_CURRENT), "focus_out": xembed(FOCUS_OUT),
    "info_mapped": [0, XEMBED_MAPPED], "info_unmapped": [0, 0],
}
assert XEMBED["embedded_notify"] == [0, 0, 0, OURS, 0] and XEMBED["focus_in"] == [0, 4, 0, 0, 0]
FOCUS = {
    "in": {"detail": NOTIFY_NONLINEAR, "heard": ["window_activate", "focus_in"]},
    "out": {"detail": NOTIFY_NONLINEAR, "heard": ["focus_out", "window_deactivate"]},
    "out_inferior": {"detail": NOTIFY_INFERIOR, "heard": []},
    "in_pointer": {"detail": NOTIFY_POINTER, "heard": []},
    "never": ["setContentScaleFactor", "onKeyDown", "onKeyUp", "onWheel", "onFocus"],
}
CLASS = "dawsome-test"
PINS = {
    "plugin_window_mask": STRUCTURE_NOTIFY, "embedder_mask": STRUCTURE_NOTIFY | SUBSTRUCTURE_NOTIFY | FOCUS_CHANGE,
    "watched_child_mask": PROPERTY_CHANGE | STRUCTURE_NOTIFY,
    "class": CLASS, "wm_class": list((CLASS + "\0" + CLASS + "\0").encode("ascii")),
}
assert (PINS["plugin_window_mask"], PINS["embedder_mask"], PINS["watched_child_mask"]) == (
    0x20000, 0x2A0000, 0x420000)
assert bytes(PINS["wm_class"]) == b"dawsome-test\0dawsome-test\0"


def wm_size_hints(size):
    """WM_NORMAL_HINTS keeping a window at `size` (ICCCM 4.1.2.3: flags PMinSize | PMaxSize,
    then 17 words, the least and the most size at words 5-8)."""
    return [16 | 32, 0, 0, 0, 0, size[0], size[1], size[0], size[1]] + [0] * 9


def configure_request(window, size):
    """A ConfigureWindow request for `size` (each side held to 1..32767), little-endian: opcode 12,
    a pad byte, the length in words (5), the window, the value-mask (width 4 | height 8), two pad
    bytes, the width and the height."""
    w, h = side(size[0]), side(size[1])
    return list(struct.pack("<BBHIHHII", 12, 0, 5, window, 4 | 8, 0, w, h))


CONFIGURE = {"window": OURS, "cases": [{"asked": list(asked), "bytes": configure_request(OURS, asked)}
                                       for asked in [(800, 600), (0, 0), (40000, 5)]]}
assert CONFIGURE["cases"][0]["bytes"] == [12, 0, 5, 0, 0x67, 0x45, 0x23, 0x01, 12, 0, 0, 0,
                                          0x20, 0x03, 0, 0, 0x58, 0x02, 0, 0]
assert CONFIGURE["cases"][1]["bytes"][-8:] == [1, 0, 0, 0, 1, 0, 0, 0]
assert CONFIGURE["cases"][2]["bytes"][-8:] == [0xFF, 0x7F, 0, 0, 5, 0, 0, 0]


# --- CLAP's test interface -------------------------------------------------------------------
CLAP_SIZE = (400, 300)
CLAP_LEAST, CLAP_MOST = (200, 150), (1600, 1200)


def clap_adjust(size):
    """The test interface's adjust_size: each side held to 200..1600 x 150..1200, then the largest
    4:3 size inside that, rounded down."""
    w = min(max(size[0], CLAP_LEAST[0]), CLAP_MOST[0])
    h = min(max(size[1], CLAP_LEAST[1]), CLAP_MOST[1])
    if w * 3 > h * 4:
        w = h * 4 // 3
    else:
        h = w * 3 // 4
    return (w, h)


def clap_open_log(fixed=False):
    return ["is_api_supported x11 embedded", "create x11 embedded", "get_size " + size_text(CLAP_SIZE),
            "can_resize " + ("no" if fixed else "yes"), "set_parent", "show"]


class ClapWindow:
    """Our window around the test interface: the size kept, our window's, the interface's."""

    def __init__(self, fixed=False):
        self.kept, self.window, self.child, self.fixed, self.log = CLAP_SIZE, CLAP_SIZE, CLAP_SIZE, fixed, []

    def plugin_request(self, asked):
        size = (side(asked[0]), side(asked[1]))
        self.log.append("request_resize %s -> true" % size_text(asked))
        self.child = size
        if size != self.kept:
            self.kept = self.window = size

    def user_resize(self, told):
        if self.fixed or told == self.kept:
            return
        held = clap_adjust(told)
        self.log.append("adjust_size %s -> %s" % (size_text(told), size_text(held)))
        if held == self.kept:
            self.window = held
            return
        self.kept = held
        self.log.append("set_size " + size_text(held))
        self.child = self.window = held
        # Our window's echo of the size it took is the one kept: nothing more.
        self.user_resize(self.window)


def clap_plugin_requests():
    window = ClapWindow()
    steps = []
    for asked in [(300, 200), (300, 200), (640, 480)]:
        window.log = []
        window.plugin_request(asked)
        steps.append({"asked": list(asked), "log": window.log, "window": list(window.window),
                      "child": list(window.child)})
    fixed = ClapWindow(fixed=True)
    fixed.plugin_request((300, 200))
    return steps, {"open_log": clap_open_log(fixed=True), "asked": [300, 200], "log": fixed.log,
                   "window": list(fixed.window), "hints_before": wm_size_hints(CLAP_SIZE),
                   "hints_after": wm_size_hints(fixed.window)}


CLAP_REQUESTS, CLAP_FIXED = clap_plugin_requests()
assert [s["log"] for s in CLAP_REQUESTS] == [["request_resize 300x200 -> true"]] * 2 + [
    ["request_resize 640x480 -> true"]]
assert CLAP_FIXED["window"] == [300, 200] and CLAP_FIXED["hints_after"][5:9] == [300, 200, 300, 200]


def clap_user_resizes(sizes, fixed_sizes=()):
    """The resizable interface's cases in turn on one window (each from the size the one before
    left), then the fixed interface's."""
    cases = []
    windows = {False: ClapWindow(), True: ClapWindow(fixed=True)}
    for fixed, size in [(False, s) for s in sizes] + [(True, s) for s in fixed_sizes]:
        window = windows[fixed]
        window.log = []
        window.user_resize(size)
        cases.append({"fixed": fixed, "size": list(size), "log": window.log, "window": list(window.window),
                      "child": list(window.child)})
    return cases


# Told by the window manager (a ConfigureNotify sent as the window manager sends one): wider than
# its aspect (Surge XT's case seen in use, 2026-10-28), taller, bigger both ways, smaller than least.
CLAP_USER_RESIZES = clap_user_resizes([(600, 300), (400, 500), (800, 700), (100, 100)], [(500, 500)])
# By hand: 600x300 holds to 4:3 inside it, 400x300, the size kept: our window back, no set_size;
# 400x500 likewise; 800x700 to 800x600; 100x100 to the least, 200x150; a fixed one nothing.
assert [c["log"] for c in CLAP_USER_RESIZES] == [
    ["adjust_size 600x300 -> 400x300"], ["adjust_size 400x500 -> 400x300"],
    ["adjust_size 800x700 -> 800x600", "set_size 800x600"],
    ["adjust_size 100x100 -> 200x150", "set_size 200x150"], []]
assert [c["window"] for c in CLAP_USER_RESIZES] == [[400, 300], [400, 300], [800, 600], [200, 150], [400, 300]]
# Asked of the window manager by another client, as `xdotool windowsize` asks it: what it gives
# comes back as the user's resize; a size the interface keeps (1000x750, 4:3) is taken as given.
CLAP_REAL_RESIZES = clap_user_resizes([(600, 300), (1000, 750), (900, 900)])
assert [c["log"] for c in CLAP_REAL_RESIZES] == [
    ["adjust_size 600x300 -> 400x300"], ["adjust_size 1000x750 -> 1000x750", "set_size 1000x750"],
    ["adjust_size 900x900 -> 900x675", "set_size 900x675"]]
CLAP_ADJUST = sorted({(tuple(c["size"]), clap_adjust(tuple(c["size"])))
                      for c in CLAP_USER_RESIZES + CLAP_REAL_RESIZES})
CLAP = {
    "id": "org.dawsome.test.gui", "name": "Dawsome Test GUI", "file": "dawsome_test_gui.clap",
    "size": list(CLAP_SIZE), "least": list(CLAP_LEAST), "most": list(CLAP_MOST),
    "open_log": clap_open_log(), "close_log": ["destroy"],
    "plugin_requests": CLAP_REQUESTS, "fixed": CLAP_FIXED,
    "user_resizes": CLAP_USER_RESIZES, "real_resizes": CLAP_REAL_RESIZES,
    "adjust": [[list(asked), list(held)] for asked, held in CLAP_ADJUST],
    "configure_request": CONFIGURE,
}

CLOSE = ["removed", "setFrame null", "view freed"]
TERMINATED = ["controller terminate", "component terminate"]

UNAVAILABLE = "Plugin windows need X11 (XWayland)"
EXPECTED = {
    "ids": {"view": VIEW, "controller": CTRL2},
    "name": NAME, "title": NAME,
    "scan_record": {"id": VIEW, "name": NAME, "vendor": "Dawsome", "version": "1.0.0", "kind": "instrument",
                    "gui": "none", "params": 5},
    "render": {"fixture": "p29_vst3_view.json", "rate": 48000, "frames": len(RENDER), "sha256": sha256_of(RENDER),
               "nonzero": sum(1 for v in RENDER if v != 0.0),
               "probes": [[k, RENDER[k]] for k in (S0 - 1, S0, S0 + 1, S1 - 1, S1)]},
    "size": list(SIZE),
    "constraint": {"least": list(LEAST), "most": list(MOST)},
    "open_log": open_log(),
    "open_heard": ["embedded_notify", "mapped"],
    "no_display": {"reason": UNAVAILABLE,
                   "log": ["createView editor", "isPlatformTypeSupported X11EmbedWindowID", "view freed"]},
    "no_editor": ["Sine", "Gain", "Fail", "Tick", "Latency"],
    "plugin_resizes": RESIZES,
    "fixed": FIXED_RESIZE, "fixed_hints": {"before": wm_size_hints(SIZE), "after": wm_size_hints((300, 200))},
    "user_resizes": USER_RESIZES,
    "run_loop": {"poked": 1, "log": ["fd pipe"]},
    "focus": FOCUS,
    "close": CLOSE, "terminated": TERMINATED,
    "cycles": 20,
    "xembed": XEMBED, "pins": PINS, "clap": CLAP,
    "app": {"opened_view": "opened Channel(ChannelId(1)): Embedded",
            "opened_sine": "opened Channel(ChannelId(1)): Generic",
            "title": NAME + " — " + NAME},
}


def files():
    out = {name + ".json": J(doc).encode() for name, doc in FIX.items()}
    out["p29_expected.json"] = (J(EXPECTED) + "\n").encode()
    return out


def main():
    args = sys.argv[1:]
    if len(args) == 2 and args[0] in ("--write", "--check"):
        stale = []
        for path, content in sorted(files().items()):
            target = os.path.join(args[1], path)
            if args[0] == "--write":
                with open(target, "wb") as f:
                    f.write(content)
            else:
                try:
                    with open(target, "rb") as f:
                        same = f.read() == content
                except OSError:
                    same = False
                if not same:
                    stale.append(path)
        if stale:
            sys.exit("not what gen_p29_expected.py writes: " + ", ".join(stale))
        if args[0] == "--check":
            return
    elif args:
        sys.exit(__doc__)
    print("class ids: view %s, controller %s" % (VIEW, CTRL2))
    print("render: sha256 %s..., probes %s" % (EXPECTED["render"]["sha256"][:16],
                                              ", ".join("%d: %.10g" % (k, v) for k, v in EXPECTED["render"]["probes"])))
    print("open: %s" % EXPECTED["open_log"])
    for step in RESIZES:
        print("plugin resize %s%s: %s" % (step["asked"], " nested" if step["nested"] else "", step["log"]))
    for case in USER_RESIZES:
        print("user resize %s%s: %s" % (case["size"], " fixed" if case["fixed"] else "", case["log"]))
    for case in CLAP_USER_RESIZES + CLAP_REAL_RESIZES:
        print("CLAP user resize %s%s: %s, window %s" % (case["size"], " fixed" if case["fixed"] else "",
                                                       case["log"], case["window"]))


if __name__ == "__main__":
    main()
