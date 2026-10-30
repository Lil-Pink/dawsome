#!/usr/bin/env python3
"""Checks the starter kit and the electronic instrument sets against their manifest, kit.json,
with the standard library only, so scripts/check.sh can run it under python3 -I on any
platform:

  each sound kit.json lists has the SHA-256 it states and is a 24-bit integer PCM WAV with the
  stated rate, channels and frames, its largest absolute sample the stated peak; each entry
  names its note ("drum" or a note such as "C2") and the CC0-1.0 license. A drum is a file of
  the starter kit, here. A pitched sound is an instrument set of its own,
  ../instruments/<set>/ (<set> is its name in lower case, spaces made hyphens): its WAV in
  samples/, and a .sfz and a set.json that are what --write-sets writes. No WAV lies here, and
  no set folder or file in ../instruments/, that kit.json does not make.

A set's .sfz is one region over every key, rooted at its note (C4 = MIDI 60), playing until
note-off (loop_mode=no_loop) and releasing over 1 s for a pad, 0.3 s for the rest. A sound may
have more roots: "zones" lists each other root's file (its note, file and the same file fields
as the sound's own), named as the sound with its note made the root's ("Bass 808 C1" rooted at
C0: "Bass 808 C0.wav"); then the .sfz has a region per root, in key order, each key played by
its nearest root (a tie by the higher), the lowest from key 0, the highest to 127. A file with a
"loop" [start, end] (frames, end inclusive) sustains on it while the key is held
(loop_mode=loop_continuous): the frames before its start are its last frames and the file goes
on past its end with its first frames, so its wrap reads on as the wave does, and the file then
ends in silence. Its set.json
names it as the Library shows it: the sound's name without its note ("Pluck C4" -> "Pluck"), since
the set plays every key and programs number octaves differently; the two 808 basses by their
register ("Bass 808 low", "Bass 808 high"); and its "category", the group the Library sorts it
into (Bass, Pads, Synth). A sound whose entry says "cut_itself": true (the two 808 basses)
has it in its set.json too: the Library adds its set as a channel on which a new note ends the
note before it, as a bass string does (any other value is refused; false is the field left out).

usage:
  python3 -I check.py              list the kit
  python3 -I check.py --check [DIR [SETS]]
                                   write nothing; exit 1, naming each file, when anything in
                                   DIR (default: this script's folder) or SETS (default:
                                   DIR/../instruments) differs from what kit.json states
  python3 -I check.py --write-sets [DIR [SETS]]
                                   write each pitched sound's .sfz and set.json into SETS

The WAVs and kit.json are made outside this repository and committed as made; this script
checks them and never makes a sound. Why hashes and not a regeneration: the synthesis and
Surge's effects are not bit-stable across versions, platforms and processes, so the kit
commits its WAVs and states what they are.
"""
import array, hashlib, json, math, os, re, struct, sys

LICENSE = "CC0-1.0"
MAKER = "Dawsome"
NOTE = re.compile(r"drum|[A-G]#?[0-9]")
FIELDS = ("name", "file", "note", "channels", "frames", "seconds", "rate", "bits", "peak",
          "peak_dbfs", "sha256", "recipe", "source", "licence")
# What each file of a sound states: the sound's own file and each of its "zones".
FILE_FIELDS = ("file", "note", "channels", "frames", "seconds", "rate", "bits", "peak",
               "peak_dbfs", "sha256")
# How many frames each side of a loop's seam must read on as the wave does (within SEAM_LSB):
# more than the reach of a click, so a wrap that jumps shows.
SEAM_FRAMES, SEAM_LSB = 64, 2
FULL_SCALE = 8388607
PITCH_CLASSES = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}
# Seconds of release after note-off: a pad fades as pads do; a bass, lead or pluck stops short
# of a click.
PAD_RELEASE, RELEASE = 1.0, 0.3
# The set names that are not the sound's name without its note: two sounds of one name an
# octave apart.
SET_NAMES = {"Bass 808 C1": "Bass 808 low", "Bass 808 C2": "Bass 808 high"}
# What each set plays, by its sound's first word: the group the Library sorts it into (set.json
# "category"); a word not here is "Other".
CATEGORIES = {"Bass": "Bass", "Pad": "Pads", "Lead": "Synth", "Pluck": "Synth"}


def set_folder(name):
    """The folder of a pitched sound's set: 'Bass 808 C1' -> 'bass-808-c1'."""
    return name.lower().replace(" ", "-")


def key_of(note):
    """The MIDI key of a note name, C4 = 60: 'C1' -> 24, 'C#4' -> 61."""
    sharp = 1 if note[1:2] == "#" else 0
    return 12 * (int(note[1 + sharp:]) + 1) + PITCH_CLASSES[note[0]] + sharp


def set_name(sound):
    """A pitched sound's set's name in the Library: 'Pluck C4' -> 'Pluck', 'Bass 808 C1' ->
    'Bass 808 low'."""
    name = sound["name"]
    suffix = " " + sound["note"]
    return SET_NAMES.get(name, name[:-len(suffix)] if name.endswith(suffix) else name)


def release(sound):
    return PAD_RELEASE if sound["name"].startswith("Pad ") else RELEASE


def zone_file(sound, note):
    """The file of a pitched sound's root `note`: 'Bass 808 C1' at 'C0' -> 'Bass 808 C0.wav'."""
    return sound["name"][:-len(sound["note"])] + note + ".wav"


def zones(sound):
    """A pitched sound's files, its own and its "zones", in key order (lowest root first)."""
    return sorted([sound] + list(sound.get("zones", [])), key=lambda z: key_of(z["note"]))


def key_ranges(roots):
    """(lokey, hikey) of each root of `roots` (MIDI keys, ascending): every key plays its
    nearest root, a tie the higher; the lowest from key 0, the highest to key 127."""
    his = [(low + high - 1) // 2 for low, high in zip(roots, roots[1:])] + [127]
    return list(zip([0] + [hi + 1 for hi in his[:-1]], his))


def set_files(sound):
    """The .sfz and set.json of a pitched sound's set, by file name, as bytes."""
    folder = set_folder(sound["name"])
    files = zones(sound)
    regions = []
    for z, (low, high) in zip(files, key_ranges([key_of(z["note"]) for z in files])):
        mode = ("loop_continuous loop_start=%d loop_end=%d" % tuple(z["loop"])
                if "loop" in z else "no_loop")
        regions.append("<region> sample=samples/%s lokey=%d hikey=%d pitch_keycenter=%d"
                       " loop_mode=%s ampeg_release=%g\n"
                       % (z["file"], low, high, key_of(z["note"]), mode, release(sound)))
    sfz = ("// %s: one of Dawsome's electronic instruments, written by assets/sounds/check.py\n"
           "// --write-sets from kit.json; CC0-1.0, see set.json.\n\n" % sound["name"]
           + "".join(regions))
    about = {"name": set_name(sound), "kind": "instrument",
             "category": CATEGORIES.get(sound["name"].split(" ")[0], "Other"), "maker": MAKER,
             "licence": LICENSE, "sfz": folder + ".sfz", "source": sound["source"]}
    if sound.get("cut_itself") is True:
        about["cut_itself"] = True
    return {folder + ".sfz": sfz.encode(), "set.json": (json.dumps(about, indent=1) + "\n").encode()}


def _int32_array():
    """An array type whose items are 4 bytes: 'i' on every common platform, else 'l'."""
    for code in "il":
        if array.array(code).itemsize == 4:
            return array.array(code)
    raise SystemExit("check: no 4-byte integer array type on this platform")


def peak_24bit(data):
    """The largest absolute value of the little-endian 24-bit samples in `data`."""
    count = len(data) // 3
    if not count:
        return 0
    wide = bytearray(4 * count)               # each sample in the top 3 bytes of an int32
    wide[1::4], wide[2::4], wide[3::4] = data[0::3], data[1::3], data[2::3]
    values = _int32_array()
    values.frombytes(bytes(wide))
    if sys.byteorder == "big":
        values.byteswap()
    return max(max(values), -min(values)) >> 8


def read_wav(data):
    """(channels, rate, bits, frames, peak, samples) of a 24-bit PCM WAV's bytes, `samples` its
    data chunk; ValueError if it is not one or is cut short."""
    if len(data) < 12 or data[:4] != b"RIFF" or data[8:12] != b"WAVE":
        raise ValueError("not a RIFF/WAVE file")
    fmt = samples = None
    pos = 12
    while pos + 8 <= len(data):
        chunk, size = data[pos:pos + 4], int.from_bytes(data[pos + 4:pos + 8], "little")
        body = data[pos + 8:pos + 8 + size]
        if len(body) < size:
            raise ValueError("its %r chunk runs past the end of the file" % chunk.decode("latin-1"))
        if chunk == b"fmt ":
            fmt = body
        elif chunk == b"data":
            samples = body
        pos += 8 + size + (size & 1)          # chunks are word-aligned
    if fmt is None or samples is None or len(fmt) < 16:
        raise ValueError("it has no fmt or data chunk")
    tag, channels, rate, byte_rate, align, bits = struct.unpack("<HHIIHH", fmt[:16])
    if tag != 1 or bits != 24:
        raise ValueError("it is not 24-bit integer PCM (format %d, %d bits)" % (tag, bits))
    if channels < 1 or align != 3 * channels or byte_rate != rate * align:
        raise ValueError("its fmt chunk is inconsistent")
    if len(samples) % align:
        raise ValueError("its data is not a whole number of frames")
    return channels, rate, bits, len(samples) // align, peak_24bit(samples), samples


def seam_problem(samples, channels, frames, loop):
    """Why the loop [start, end] of a file's 24-bit `samples` is not one its wrap reads on
    through, or None: the SEAM_FRAMES frames before its start must equal its last ones, the file
    must go on past its end for SEAM_FRAMES frames that equal its first ones (each within
    SEAM_LSB), and its last frame must be silence (within SEAM_LSB)."""
    if not (isinstance(loop, list) and len(loop) == 2 and all(isinstance(v, int) for v in loop)):
        return "its loop %r is not [start, end]" % (loop,)
    start, end = loop
    if not SEAM_FRAMES <= start <= end < frames - SEAM_FRAMES:
        return ("its loop %d..%d does not leave %d frames before it and after it in its %d"
                % (start, end, SEAM_FRAMES, frames))

    def value(frame, channel):
        at = 3 * (frame * channels + channel)
        return int.from_bytes(samples[at:at + 3], "little", signed=True)

    def worst(a, b):
        return max(abs(value(a + i, c) - value(b + i, c))
                   for i in range(SEAM_FRAMES) for c in range(channels))

    before, after = worst(start - SEAM_FRAMES, end + 1 - SEAM_FRAMES), worst(end + 1, start)
    if max(before, after) > SEAM_LSB:
        return ("its loop's seam differs from the wave by up to %d LSB (%d before its start, "
                "%d after its end): its wrap clicks" % (max(before, after), before, after))
    if max(abs(value(frames - 1, c)) for c in range(channels)) > SEAM_LSB:
        return "it does not end in silence after its loop"
    return None


def read_manifest(folder):
    """kit.json in `folder` and its sounds; ValueError when it cannot be read as the manifest."""
    try:
        with open(os.path.join(folder, "kit.json"), encoding="utf-8") as f:
            manifest = json.load(f)
        return manifest, list(manifest["sounds"])
    except (OSError, ValueError, KeyError, TypeError) as e:
        raise ValueError("kit.json: cannot be read as the kit's manifest (%s)" % e)


def file_problems(folder, manifest, record, path):
    """Every way one file of a sound (`record`: the sound itself or one of its zones, lying at
    `path`) differs from what kit.json states, naming it by its path from `folder`."""
    name = os.path.relpath(path, folder)
    found = []
    if (record["rate"], record["bits"]) != (manifest.get("rate"), manifest.get("bits")):
        found.append("%s: rate or bits differ from the kit's" % name)
    try:
        with open(path, "rb") as f:
            data = f.read()
    except OSError:
        return found + ["%s: missing" % name]
    if hashlib.sha256(data).hexdigest() != record["sha256"]:
        found.append("%s: its SHA-256 is not the one kit.json states" % name)
    try:
        channels, rate, bits, frames, peak, samples = read_wav(data)
    except ValueError as e:
        return found + ["%s: %s" % (name, e)]
    stated = (record["channels"], record["rate"], record["bits"], record["frames"], record["peak"])
    if (channels, rate, bits, frames, peak) != stated:
        found.append("%s: holds %d ch, %d Hz, %d bits, %d frames, peak %d; kit.json states "
                     "%d ch, %d Hz, %d bits, %d frames, peak %d"
                     % ((name, channels, rate, bits, frames, peak) + stated))
    if frames and record["seconds"] != round(frames / rate, 3):
        found.append("%s: seconds %r is not frames / rate" % (name, record["seconds"]))
    # log10 may differ in its last bit between platforms' maths libraries: compare loosely.
    if peak and abs(record["peak_dbfs"] - 20 * math.log10(peak / FULL_SCALE)) > 0.001:
        found.append("%s: peak_dbfs %r is not the peak's" % (name, record["peak_dbfs"]))
    if "loop" in record:
        problem = seam_problem(samples, channels, frames, record["loop"])
        if problem:
            found.append("%s: %s" % (name, problem))
    return found


def zones_problem(sound):
    """Why a pitched sound's "zones" are not a list of files with their fields and note names,
    or None."""
    others = sound.get("zones", [])
    if not isinstance(others, list) or not all(isinstance(z, dict) for z in others):
        return "its zones are not a list of files"
    for z in others:
        lacks = [k for k in FILE_FIELDS if k not in z]
        if lacks:
            return "a zone lacks %s" % ", ".join(lacks)
        if not isinstance(z["note"], str) or not NOTE.fullmatch(z["note"]) or z["note"] == "drum":
            return "a zone's note %r is not a note name" % (z["note"],)
    return None


def sound_problems(folder, sets, manifest, sound):
    """Every way one sound kit.json lists differs from what it states, naming the file by its
    path from `folder`."""
    name = sound.get("file", "?") if isinstance(sound, dict) else "?"
    missing = [k for k in FIELDS if not isinstance(sound, dict) or k not in sound]
    if missing:
        return ["%s: kit.json's entry lacks %s" % (name, ", ".join(missing))]
    found = []
    if sound["file"] != sound["name"] + ".wav":
        found.append("%s: its file is not named after its sound (%s.wav)" % (name, sound["name"]))
    if not isinstance(sound["note"], str) or not NOTE.fullmatch(sound["note"]):
        return found + ["%s: note %r is neither \"drum\" nor a note name" % (name, sound["note"])]
    pitched = sound["note"] != "drum"
    if sound["licence"] != LICENSE:
        found.append("%s: license %r is not %s" % (name, sound["licence"], LICENSE))
    if "cut_itself" in sound and sound["cut_itself"] is not True:
        found.append("%s: cut_itself %r is not true (leave it out for off)"
                     % (name, sound["cut_itself"]))
    if not pitched:
        if "zones" in sound or "loop" in sound or "cut_itself" in sound:
            found.append("%s: a drum has no zones, loop or cut_itself" % name)
        return found + file_problems(folder, manifest, sound, os.path.join(folder, sound["file"]))
    shape = zones_problem(sound)
    if shape:
        return found + ["%s: %s" % (name, shape)]
    others = sound.get("zones", [])
    for z in others:
        if z["file"] != zone_file(sound, z["note"]):
            found.append("%s: its %s zone is not named %s" % (name, z["note"],
                                                               zone_file(sound, z["note"])))
    notes = [key_of(z["note"]) for z in [sound] + others]
    if len(set(notes)) != len(notes):
        return found + ["%s: two of its zones share a root" % name]
    set_dir = os.path.join(sets, set_folder(sound["name"]))
    for file, made in set_files(sound).items():
        target = os.path.join(set_dir, file)
        try:
            with open(target, "rb") as f:
                same = f.read() == made
        except OSError:
            same = False
        if not same:
            found.append("%s: not what check.py --write-sets writes"
                         % os.path.relpath(target, folder))
    for z in zones(sound):
        found += file_problems(folder, manifest, z, os.path.join(set_dir, "samples", z["file"]))
    return found


def strays(folder, sets, sounds):
    """The WAVs in `folder` and the folders and files in `sets` that kit.json does not make."""
    drums = {s.get("file") for s in sounds if isinstance(s, dict) and s.get("note") == "drum"}
    made = {}
    for sound in sounds:
        whole = isinstance(sound, dict) and all(k in sound for k in FIELDS)
        pitched = whole and NOTE.fullmatch(str(sound["note"])) and sound["note"] != "drum"
        if pitched and not zones_problem(sound):
            files = ({"samples"} | {"samples/" + z["file"] for z in zones(sound)}
                     | set(set_files(sound)))
            made[set_folder(sound["name"])] = files
    found = ["%s: not in kit.json" % file for file in sorted(os.listdir(folder))
             if file.lower().endswith(".wav") and file not in drums]
    if not os.path.isdir(sets):
        return found + ["%s: missing" % os.path.relpath(sets, folder)] if made else found
    for entry in sorted(os.listdir(sets)):
        path = os.path.join(sets, entry)
        if entry not in made:
            found.append("%s: a set kit.json does not make" % os.path.relpath(path, folder))
            continue
        for root, folders, files in os.walk(path):
            for part in sorted(folders + files):
                inside = os.path.relpath(os.path.join(root, part), path).replace(os.sep, "/")
                if inside not in made[entry]:
                    found.append("%s: not made from kit.json"
                                 % os.path.relpath(os.path.join(root, part), folder))
    return found


def problems(folder, sets):
    """Every way `folder` and the sets in `sets` differ from kit.json, one line each, naming the
    file by its path from `folder`."""
    try:
        manifest, sounds = read_manifest(folder)
    except ValueError as e:
        return [str(e)]
    found = []
    if manifest.get("licence") != LICENSE:
        found.append("kit.json: the kit's license is not %s" % LICENSE)
    for sound in sounds:
        found += sound_problems(folder, sets, manifest, sound)
    files = [s.get("file") for s in sounds if isinstance(s, dict)]
    if len(set(files)) != len(files):
        found.append("kit.json: a file is listed twice")
    return found + strays(folder, sets, sounds)


def write_sets(folder, sets):
    """Writes each pitched sound's .sfz and set.json into its set's folder in `sets`."""
    _, sounds = read_manifest(folder)
    for sound in sounds:
        if sound["note"] == "drum":
            continue
        set_dir = os.path.join(sets, set_folder(sound["name"]))
        os.makedirs(set_dir, exist_ok=True)
        for file, data in set_files(sound).items():
            with open(os.path.join(set_dir, file), "wb") as f:
                f.write(data)


def main():
    args = sys.argv[1:]
    here = os.path.dirname(os.path.abspath(__file__))
    if args[:1] in (["--check"], ["--write-sets"]) and len(args) <= 3:
        folder = args[1] if len(args) >= 2 else here
        sets = args[2] if len(args) == 3 else os.path.join(folder, "..", "instruments")
        if args[0] == "--write-sets":
            try:
                write_sets(folder, sets)
            except (ValueError, KeyError, OSError) as e:
                sys.exit("check: %s" % e)
            return
        found = problems(folder, sets)
        for line in found:
            print("check: %s" % (line if line.startswith("kit.json")
                                 else os.path.normpath(os.path.join(folder, line))),
                  file=sys.stderr)
        sys.exit(1 if found else 0)
    if args:
        sys.exit(__doc__)
    with open(os.path.join(here, "kit.json"), encoding="utf-8") as f:
        for sound in json.load(f)["sounds"]:
            where = "starter kit" if sound["note"] == "drum" else "set " + set_folder(sound["name"])
            print("%-14s %-5s %d ch  %6.3f s  peak %.2f dBFS  %s" % (
                sound["name"], sound["note"], sound["channels"], sound["seconds"],
                sound["peak_dbfs"], where))


if __name__ == "__main__":
    main()
