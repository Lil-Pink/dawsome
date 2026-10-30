//! Reads the part of the SFZ format instrument sets are written in: the `<control>`,
//! `<global>`, `<master>`, `<group>` and `<region>` headers with their inheritance, `#define`
//! and `#include`, comments, and the opcodes [`Region`] holds. Everything else is left out and
//! listed, each opcode or header once with how often and where it first appears, so a set that
//! leans on more than this reader plays is visible as such.
//!
//! No I/O: the caller reads the files, and `#include` reaches them through a function it
//! passes in, the way the project-file import finds its samples.

use std::fmt;

/// How a region's sound continues while and after its note is held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    /// Plays from its start to its end, and stops at note-off.
    NoLoop,
    /// Plays from its start to its end, whatever the note does.
    OneShot,
    /// Loops between its loop points for as long as it sounds, its release included.
    Continuous,
    /// Loops while the note is held, then plays on from where it was.
    Sustain,
}

impl LoopMode {
    /// The mode an SFZ value names; `continuous` too, which Karoryfer's sets write.
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "no_loop" => LoopMode::NoLoop,
            "one_shot" => LoopMode::OneShot,
            "loop_continuous" | "continuous" => LoopMode::Continuous,
            "loop_sustain" => LoopMode::Sustain,
            _ => return None,
        })
    }
}

/// One region as its headers and opcodes define it, the levels above it folded in.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// The sample's path relative to the set's main file, `/`-separated, `default_path` in
    /// front.
    pub sample: String,
    pub lokey: u8,
    pub hikey: u8,
    pub pitch_keycenter: u8,
    pub lovel: u8,
    pub hivel: u8,
    /// `None` when the file gives none: then loop points alone mean a continuous loop.
    pub loop_mode: Option<LoopMode>,
    /// In frames of the sample file, `loop_end` inclusive.
    pub loop_start: Option<u64>,
    pub loop_end: Option<u64>,
    /// Seconds.
    pub ampeg_attack: f64,
    pub ampeg_release: f64,
    /// Percent, -100 to 100.
    pub amp_veltrack: f64,
    /// Decibels, from -144 to 6 ([`VOLUME_DB`]).
    pub volume: f64,
    /// Cents.
    pub tune: f64,
    /// Semitones.
    pub transpose: i32,
    /// -100 (left) to 100 (right).
    pub pan: f64,
    /// The first frame it plays.
    pub offset: u64,
    /// The last frame it plays, inclusive; `None` plays to the file's end.
    pub end: Option<u64>,
    pub seq_length: u32,
    pub seq_position: u32,
    /// Where its `<region>` header is.
    pub at: Location,
}

impl Region {
    /// A region of `sample` with every opcode at its default.
    fn new(at: Location) -> Self {
        Self {
            sample: String::new(),
            lokey: 0,
            hikey: 127,
            pitch_keycenter: 60,
            lovel: 1,
            hivel: 127,
            loop_mode: None,
            loop_start: None,
            loop_end: None,
            ampeg_attack: 0.0,
            ampeg_release: 0.0,
            amp_veltrack: 100.0,
            volume: 0.0,
            tune: 0.0,
            transpose: 0,
            pan: 0.0,
            offset: 0,
            end: None,
            seq_length: 1,
            seq_position: 1,
            at,
        }
    }

    fn apply(&mut self, op: &Op) {
        match op {
            Op::Sample(path) => self.sample.clone_from(path),
            Op::LoKey(key) => self.lokey = *key,
            Op::HiKey(key) => self.hikey = *key,
            Op::Key(key) => (self.lokey, self.hikey, self.pitch_keycenter) = (*key, *key, *key),
            Op::KeyCenter(key) => self.pitch_keycenter = *key,
            Op::LoVel(velocity) => self.lovel = *velocity,
            Op::HiVel(velocity) => self.hivel = *velocity,
            Op::LoopMode(mode) => self.loop_mode = Some(*mode),
            Op::LoopStart(frame) => self.loop_start = Some(*frame),
            Op::LoopEnd(frame) => self.loop_end = Some(*frame),
            Op::Attack(seconds) => self.ampeg_attack = *seconds,
            Op::Release(seconds) => self.ampeg_release = *seconds,
            Op::VelTrack(percent) => self.amp_veltrack = *percent,
            Op::Volume(db) => self.volume = *db,
            Op::Tune(cents) => self.tune = *cents,
            Op::Transpose(semitones) => self.transpose = *semitones,
            Op::Pan(pan) => self.pan = *pan,
            Op::Offset(frame) => self.offset = *frame,
            Op::End(frame) => self.end = u64::try_from(*frame).ok(),
            Op::SeqLength(length) => self.seq_length = *length,
            Op::SeqPosition(position) => self.seq_position = *position,
        }
    }
}

/// A line of a file of the set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The main file's name, or an included file's path as its `#include` writes it.
    pub file: String,
    /// Counted from 1.
    pub line: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.file, self.line)
    }
}

/// An opcode, or a header (written `<name>`), this reader leaves out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    pub name: String,
    pub count: usize,
    pub first: Location,
}

/// Something in the text that could not be read as written; what it named was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub at: Location,
    pub text: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.at, self.text)
    }
}

/// A set's regions in file order, and what reading it left out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Sfz {
    pub regions: Vec<Region>,
    /// In the order first met.
    pub ignored: Vec<Ignored>,
    pub problems: Vec<Problem>,
}

/// How deep `#include`s may nest. Why a limit at all: a file that includes itself through
/// another would otherwise never end.
const MAX_INCLUDE_DEPTH: usize = 16;

/// The most files `#include`s may read for one set, and the most text a set's files may hold
/// together (bytes). Why: files that include each other many times over (one including another
/// ten times, that one the next ten times, or one file under ever new spellings) multiply past
/// anything the depth limit bounds, and one such file took the whole program down; no set
/// written by hand comes near either figure.
pub const MAX_INCLUDE_READS: usize = 256;
pub const MAX_SET_BYTES: usize = 16 << 20;

/// The most problems listed for one set; one more line counts the rest. Why: a file of junk
/// would list a problem per line, and a person reads the report.
pub const MAX_PROBLEMS: usize = 1000;

/// A file of a set as the caller read it: what makes it that file whatever path spelled it
/// (the caller's canonical path), and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetFile {
    pub id: String,
    pub text: String,
}

/// Reads the set whose main file `file` holds `text`; `include` gives the text of a file an
/// `#include` names (its path as written, relative to the main file's folder) or why it cannot.
/// Each spelling of a path counts as a file of its own; [`parse_files`] knows one file under
/// two spellings.
pub fn parse(
    file: &str,
    text: &str,
    include: &mut dyn FnMut(&str) -> Result<String, String>,
) -> Sfz {
    parse_files(file, file, text, &mut |path| {
        include(path).map(|text| SetFile {
            id: path.to_owned(),
            text,
        })
    })
}

/// [`parse`] with each file known by its identity: the main file `file` is `id`, and `include`
/// gives each included file's. A file that includes one being read, under any spelling, is
/// refused; at most [`MAX_INCLUDE_READS`] includes are read and [`MAX_SET_BYTES`] of text in
/// all, the first one past either reported and the rest left out quietly.
pub fn parse_files(
    file: &str,
    id: &str,
    text: &str,
    include: &mut dyn FnMut(&str) -> Result<SetFile, String>,
) -> Sfz {
    let mut reader = Reader {
        include,
        defines: Vec::new(),
        default_path: String::new(),
        levels: Default::default(),
        region: None,
        header: Header::None,
        open: Vec::new(),
        reads: 0,
        bytes: text.len(),
        spent: false,
        dropped: None,
        out: Sfz::default(),
    };
    reader.file(file, id, text);
    reader.flush();
    if let Some((at, more)) = reader.dropped {
        reader.out.problems.push(Problem {
            at,
            text: format!("and {more} more problems from here on"),
        });
    }
    reader.out
}

/// A supported opcode with its value read.
#[derive(Debug, Clone, PartialEq)]
enum Op {
    Sample(String),
    LoKey(u8),
    HiKey(u8),
    Key(u8),
    KeyCenter(u8),
    LoVel(u8),
    HiVel(u8),
    LoopMode(LoopMode),
    LoopStart(u64),
    LoopEnd(u64),
    Attack(f64),
    Release(f64),
    VelTrack(f64),
    Volume(f64),
    Tune(f64),
    Transpose(i32),
    Pan(f64),
    Offset(u64),
    End(i64),
    SeqLength(u32),
    SeqPosition(u32),
}

/// Which header the opcodes being read belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Header {
    /// Before the first header.
    None,
    Control,
    /// An inheritance level: 0 global, 1 master, 2 group.
    Level(usize),
    Region,
    /// A header this reader leaves out, with everything under it.
    Ignored,
}

struct Reader<'a> {
    include: &'a mut dyn FnMut(&str) -> Result<SetFile, String>,
    /// `#define`d names (without the `$`) and their values, in the order defined.
    defines: Vec<(String, String)>,
    default_path: String,
    /// The opcodes of the open `<global>`, `<master>` and `<group>`.
    levels: [Vec<Op>; 3],
    /// The open region: its header's place and its own opcodes.
    region: Option<(Location, Vec<Op>)>,
    header: Header,
    /// The identities of the files being read, outermost first.
    open: Vec<String>,
    /// Includes read, and the bytes of text read, the main file's included.
    reads: usize,
    bytes: usize,
    /// Whether an include went past the budget: the rest are left out without a word each.
    spent: bool,
    /// Where the first problem past [`MAX_PROBLEMS`] is, and how many there are.
    dropped: Option<(Location, usize)>,
    out: Sfz,
}

impl Reader<'_> {
    fn file(&mut self, name: &str, id: &str, text: &str) {
        self.open.push(id.to_owned());
        let mut in_comment = false;
        for (index, raw) in text.lines().enumerate() {
            let at = Location {
                file: name.to_owned(),
                line: index + 1,
            };
            let line = strip_comments(raw, &mut in_comment);
            let trimmed = line.trim();
            if let Some(directive) = trimmed.strip_prefix('#') {
                self.directive(directive, &at);
            } else if !trimmed.is_empty() {
                let line = self.substitute(&line, &at);
                self.tokens(&line, &at);
            }
        }
        self.open.pop();
    }

    fn directive(&mut self, directive: &str, at: &Location) {
        if let Some(rest) = directive.strip_prefix("define") {
            let rest = rest.trim();
            let (name, value) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            match name.strip_prefix('$').filter(|name| is_name(name)) {
                Some(name) => {
                    let value = value.trim().to_owned();
                    match self.defines.iter_mut().find(|(known, _)| known == name) {
                        Some(defined) => defined.1 = value,
                        None => self.defines.push((name.to_owned(), value)),
                    }
                }
                None => self.problem(at, format!("#define {rest}: a name starts with $")),
            }
        } else if let Some(rest) = directive.strip_prefix("include") {
            let rest = self.substitute(rest, at);
            let path = rest.trim().trim_matches('"').replace('\\', "/");
            self.include(&path, at);
        } else {
            self.problem(
                at,
                format!("#{directive} is not a directive this reader knows"),
            );
        }
    }

    fn include(&mut self, path: &str, at: &Location) {
        if self.spent {
            return;
        }
        if self.open.len() >= MAX_INCLUDE_DEPTH {
            let text = format!("#include \"{path}\" nests deeper than {MAX_INCLUDE_DEPTH} files");
            self.problem(at, text);
            return;
        }
        if self.reads == MAX_INCLUDE_READS {
            self.spend(
                at,
                format!(
                    "#include \"{path}\": the set has read {MAX_INCLUDE_READS} included files, \
                     the most one set may; the rest are left out"
                ),
            );
            return;
        }
        self.reads += 1;
        let file = match (self.include)(path) {
            Ok(file) => file,
            Err(reason) => {
                self.problem(at, format!("#include \"{path}\": {reason}"));
                return;
            }
        };
        if self.open.contains(&file.id) {
            self.problem(
                at,
                format!("#include \"{path}\" includes a file being read"),
            );
        } else if self.bytes + file.text.len() > MAX_SET_BYTES {
            self.spend(
                at,
                format!(
                    "#include \"{path}\": the set's files would hold more than {} MB of text, \
                     the most one set may; the rest are left out",
                    MAX_SET_BYTES >> 20
                ),
            );
        } else {
            self.bytes += file.text.len();
            self.file(path, &file.id, &file.text);
        }
    }

    /// Reports the include that went past the budget; the ones after it are left out quietly.
    fn spend(&mut self, at: &Location, text: String) {
        self.spent = true;
        self.problem(at, text);
    }

    /// `line` with every `$name` defined so far replaced by its value; a name not defined is
    /// left as written and reported.
    fn substitute(&mut self, line: &str, at: &Location) -> String {
        let mut out = String::with_capacity(line.len());
        let mut rest = line;
        while let Some(dollar) = rest.find('$') {
            out.push_str(&rest[..dollar]);
            let after = &rest[dollar + 1..];
            let length = after
                .find(|c: char| !is_name_char(c))
                .unwrap_or(after.len());
            let name = &after[..length];
            match self.defines.iter().find(|(known, _)| known == name) {
                Some((_, value)) => out.push_str(value),
                None => {
                    out.push('$');
                    out.push_str(name);
                    self.problem(at, format!("${name} is not defined"));
                }
            }
            rest = &after[length..];
        }
        out.push_str(rest);
        out
    }

    /// The headers and opcodes of one line, in order. An opcode's value runs to the next
    /// header or opcode (or the line's end), trimmed, so a sample's name may hold spaces.
    fn tokens(&mut self, line: &str, at: &Location) {
        let marks = marks(line);
        if let Some(&(first, _)) = marks.first() {
            if !line[..first].trim().is_empty() {
                self.problem(
                    at,
                    format!("'{}' is not a header or an opcode", line[..first].trim()),
                );
            }
        } else {
            self.problem(
                at,
                format!("'{}' is not a header or an opcode", line.trim()),
            );
        }
        for (index, &(start, mark)) in marks.iter().enumerate() {
            let next = marks.get(index + 1).map_or(line.len(), |&(next, _)| next);
            match mark {
                Mark::Header { end } => {
                    let name = line[start + 1..end - 1].trim().to_owned();
                    self.header(&name, at);
                    let stray = line[end..next].trim();
                    if !stray.is_empty() {
                        self.problem(at, format!("'{stray}' is not a header or an opcode"));
                    }
                }
                Mark::Opcode { equals } => {
                    let name = &line[start..equals];
                    let value = line[equals + 1..next].trim();
                    self.opcode(name, value, at);
                }
            }
        }
    }

    fn header(&mut self, name: &str, at: &Location) {
        self.flush();
        self.header = match name {
            "control" => Header::Control,
            "global" => {
                self.levels = Default::default();
                Header::Level(0)
            }
            "master" => {
                self.levels[1].clear();
                self.levels[2].clear();
                Header::Level(1)
            }
            "group" => {
                self.levels[2].clear();
                Header::Level(2)
            }
            "region" => {
                self.region = Some((at.clone(), Vec::new()));
                Header::Region
            }
            other => {
                self.ignore(&format!("<{other}>"), at);
                Header::Ignored
            }
        };
    }

    fn opcode(&mut self, name: &str, value: &str, at: &Location) {
        match self.header {
            Header::Ignored => {}
            // A Windows-made set writes the folder beside it as `\samples\`: its leading
            // backslashes name no drive's root, so they go (a leading `/` stays, a path outside
            // the set, which the loader refuses).
            Header::Control if name == "default_path" => {
                self.default_path = value.trim_start_matches('\\').replace('\\', "/");
            }
            Header::Control => self.ignore(name, at),
            Header::None => {
                self.problem(at, format!("{name}={value} comes before any header"));
            }
            Header::Level(_) | Header::Region => {
                let op = match self.read_op(name, value) {
                    None => return self.ignore(name, at),
                    Some(Ok(op)) => op,
                    Some(Err(fault)) => {
                        self.problem(at, format!("{name}={value}: {}", fault.reason));
                        match fault.kept {
                            Some(op) => op,
                            None => return,
                        }
                    }
                };
                match (self.header, &mut self.region) {
                    (Header::Region, Some((_, ops))) => ops.push(op),
                    (Header::Level(level), _) => self.levels[level].push(op),
                    _ => {}
                }
            }
        }
    }

    /// `value` read for opcode `name`; `None` when this reader leaves the opcode out.
    fn read_op(&self, name: &str, value: &str) -> Option<Result<Op, Fault>> {
        if name == "volume" {
            return Some(volume(value));
        }
        let read = match name {
            "sample" if value.is_empty() => Err("names no file".to_owned()),
            "sample" => Ok(Op::Sample(
                format!("{}{value}", self.default_path).replace('\\', "/"),
            )),
            "lokey" => key(value).map(Op::LoKey),
            "hikey" => key(value).map(Op::HiKey),
            "key" => key(value).map(Op::Key),
            "pitch_keycenter" => key(value).map(Op::KeyCenter),
            "lovel" => whole(value, 0, 127).map(|v| Op::LoVel(v as u8)),
            "hivel" => whole(value, 0, 127).map(|v| Op::HiVel(v as u8)),
            "loop_mode" => LoopMode::parse(value).map(Op::LoopMode).ok_or_else(|| {
                "not one of no_loop, one_shot, loop_continuous, loop_sustain".to_owned()
            }),
            "loop_start" => whole(value, 0, i64::from(u32::MAX)).map(|v| Op::LoopStart(v as u64)),
            "loop_end" => whole(value, 0, i64::from(u32::MAX)).map(|v| Op::LoopEnd(v as u64)),
            "offset" => whole(value, 0, i64::from(u32::MAX)).map(|v| Op::Offset(v as u64)),
            "end" => whole(value, -1, i64::from(u32::MAX)).map(Op::End),
            "ampeg_attack" => number(value, 0.0, 100.0).map(Op::Attack),
            "ampeg_release" => number(value, 0.0, 100.0).map(Op::Release),
            "amp_veltrack" => number(value, -100.0, 100.0).map(Op::VelTrack),
            "tune" => number(value, -9600.0, 9600.0).map(Op::Tune),
            "transpose" => whole(value, -127, 127).map(|v| Op::Transpose(v as i32)),
            "pan" => number(value, -100.0, 100.0).map(Op::Pan),
            "seq_length" => whole(value, 1, 100).map(|v| Op::SeqLength(v as u32)),
            "seq_position" => whole(value, 1, 100).map(|v| Op::SeqPosition(v as u32)),
            _ => return None,
        };
        Some(read.map_err(Fault::left_out))
    }

    /// Ends the open region: its levels' opcodes, then its own, in order, later ones winning.
    /// A region with no sample, or with a key or velocity range upside down, is reported and
    /// left out; one whose `end` is -1 never plays, as the format says, and is left out
    /// quietly.
    fn flush(&mut self) {
        let Some((at, ops)) = self.region.take() else {
            return;
        };
        let mut region = Region::new(at);
        let all = self.levels.iter().flatten().chain(&ops);
        let mut silent = false;
        for op in all {
            region.apply(op);
            if let Op::End(end) = op {
                silent = *end < 0;
            }
        }
        let fault = if region.sample.is_empty() {
            Some("the region names no sample; it is left out".to_owned())
        } else if region.lokey > region.hikey {
            Some(format!(
                "hikey={} is below lokey={}; the region never plays and is left out",
                region.hikey, region.lokey
            ))
        } else if region.lovel > region.hivel {
            Some(format!(
                "hivel={} is below lovel={}; the region never plays and is left out",
                region.hivel, region.lovel
            ))
        } else {
            None
        };
        match fault {
            Some(text) => {
                let at = region.at.clone();
                self.problem(&at, text);
            }
            None if !silent => self.out.regions.push(region),
            None => {}
        }
    }

    fn ignore(&mut self, name: &str, at: &Location) {
        match self
            .out
            .ignored
            .iter_mut()
            .find(|ignored| ignored.name == name)
        {
            Some(ignored) => ignored.count += 1,
            None => self.out.ignored.push(Ignored {
                name: name.to_owned(),
                count: 1,
                first: at.clone(),
            }),
        }
    }

    fn problem(&mut self, at: &Location, text: String) {
        if self.out.problems.len() < MAX_PROBLEMS {
            self.out.problems.push(Problem {
                at: at.clone(),
                text,
            });
        } else {
            self.dropped.get_or_insert_with(|| (at.clone(), 0)).1 += 1;
        }
    }
}

/// A header or an opcode's start in a line.
#[derive(Debug, Clone, Copy)]
enum Mark {
    /// `<name>`, its closing `>` just before `end`.
    Header { end: usize },
    /// `name=`, the `=` at `equals`.
    Opcode { equals: usize },
}

/// Where each header and opcode of `line` starts. An opcode is a name and `=` at the start of
/// a word: after a space, a header or the line's start.
fn marks(line: &str) -> Vec<(usize, Mark)> {
    let bytes = line.as_bytes();
    let mut marks = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'<' {
            if let Some(close) = line[index..].find('>') {
                let end = index + close + 1;
                marks.push((index, Mark::Header { end }));
                index = end;
                continue;
            }
        }
        let word_start =
            index == 0 || bytes[index - 1].is_ascii_whitespace() || bytes[index - 1] == b'>';
        if word_start && is_name_char(char::from(bytes[index])) {
            let length = bytes[index..]
                .iter()
                .position(|&byte| !is_name_char(char::from(byte)))
                .unwrap_or(bytes.len() - index);
            let equals = index + length;
            if bytes.get(equals) == Some(&b'=') {
                marks.push((index, Mark::Opcode { equals }));
                index = equals + 1;
                continue;
            }
        }
        index += 1;
    }
    marks
}

/// `raw` without its `//` comment and the parts of it inside `/* */` (which may span lines:
/// `in_comment` carries that from line to line).
fn strip_comments(raw: &str, in_comment: &mut bool) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    loop {
        if *in_comment {
            match rest.find("*/") {
                Some(close) => {
                    *in_comment = false;
                    rest = &rest[close + 2..];
                    out.push(' ');
                }
                None => return out,
            }
        }
        let line_comment = rest.find("//");
        let block = rest.find("/*");
        match (line_comment, block) {
            (Some(line), Some(block)) if line < block => {
                out.push_str(&rest[..line]);
                return out;
            }
            (_, Some(block)) => {
                out.push_str(&rest[..block]);
                *in_comment = true;
                rest = &rest[block + 2..];
            }
            (Some(line), None) => {
                out.push_str(&rest[..line]);
                return out;
            }
            (None, None) => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(is_name_char)
}

/// A whole number from `lo` to `hi`.
fn whole(value: &str, lo: i64, hi: i64) -> Result<i64, String> {
    let number: i64 = value.parse().map_err(|_| "not a whole number".to_owned())?;
    if (lo..=hi).contains(&number) {
        Ok(number)
    } else {
        Err(format!("outside {lo} to {hi}"))
    }
}

/// What is wrong with an opcode's value, reported where it is: the value is left out, or, for
/// one brought into its range, `kept` as that.
struct Fault {
    reason: String,
    kept: Option<Op>,
}

impl Fault {
    fn left_out(reason: String) -> Self {
        Self { reason, kept: None }
    }
}

/// SFZ's range of `volume`, in decibels.
pub const VOLUME_DB: (f64, f64) = (-144.0, 6.0);

/// A volume, a number in [`VOLUME_DB`]; one past it plays at the range's nearer end, reported
/// (libraries write +9 and +12 dB, and the set should still sound, as loud as this reader goes).
fn volume(value: &str) -> Result<Op, Fault> {
    let (lo, hi) = VOLUME_DB;
    let db: f64 = value
        .parse()
        .map_err(|_| Fault::left_out("not a number".to_owned()))?;
    if db.is_nan() {
        return Err(Fault::left_out("not a number".to_owned()));
    }
    if (lo..=hi).contains(&db) {
        return Ok(Op::Volume(db));
    }
    let kept = db.clamp(lo, hi);
    Err(Fault {
        reason: format!("outside {lo} to {hi}; it plays at {kept}"),
        kept: Some(Op::Volume(kept)),
    })
}

/// A finite number from `lo` to `hi`.
fn number(value: &str, lo: f64, hi: f64) -> Result<f64, String> {
    let number: f64 = value.parse().map_err(|_| "not a number".to_owned())?;
    if (lo..=hi).contains(&number) {
        Ok(number)
    } else {
        Err(format!("outside {lo} to {hi}"))
    }
}

/// A key: a MIDI number from 0 to 127, or a note name with its octave (`c4` = 60, `c#4` =
/// `db4` = 61, `c-1` = 0).
fn key(value: &str) -> Result<u8, String> {
    if let Ok(number) = whole(value, 0, 127) {
        return Ok(number as u8);
    }
    let lower = value.to_ascii_lowercase();
    let mut chars = lower.chars();
    let class: i64 = match chars.next() {
        Some('c') => 0,
        Some('d') => 2,
        Some('e') => 4,
        Some('f') => 5,
        Some('g') => 7,
        Some('a') => 9,
        Some('b') => 11,
        _ => return Err("not a key from 0 to 127 or a note name".to_owned()),
    };
    let rest = chars.as_str();
    let (shift, octave) = match rest.strip_prefix('#') {
        Some(octave) => (1, octave),
        None => match rest.strip_prefix('b') {
            Some(octave) if !octave.is_empty() => (-1, octave),
            _ => (0, rest),
        },
    };
    let octave: i64 = octave
        .parse()
        .map_err(|_| "not a key from 0 to 127 or a note name".to_owned())?;
    let number = (octave + 1) * 12 + class + shift;
    u8::try_from(number)
        .ok()
        .filter(|&key| key <= 127)
        .ok_or_else(|| "a note outside 0 to 127".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Result<String, String> {
        Err("no such file".to_owned())
    }

    fn read(text: &str) -> Sfz {
        parse("test.sfz", text, &mut none)
    }

    #[test]
    fn note_names_count_from_c_minus_1() {
        assert_eq!(key("c4"), Ok(60));
        assert_eq!(key("C#4"), Ok(61));
        assert_eq!(key("db4"), Ok(61));
        assert_eq!(key("b3"), Ok(59));
        assert_eq!(key("c-1"), Ok(0));
        assert_eq!(key("g9"), Ok(127));
        assert!(key("g#9").is_err());
        assert!(key("h4").is_err());
        assert_eq!(key("72"), Ok(72));
    }

    // A value runs to the next opcode, so a sample's name keeps its spaces; a header may
    // share its line with opcodes, and levels fold in, the region's own opcodes last.
    #[test]
    fn values_run_to_the_next_opcode_and_levels_fold_in() {
        let sfz = read(
            "<group> lokey=10 hikey=20\n<region>sample=My Piano C4.wav  hikey=15 key=12 pitch_keycenter=11\n",
        );
        assert_eq!(sfz.problems, []);
        let region = &sfz.regions[0];
        assert_eq!(region.sample, "My Piano C4.wav");
        assert_eq!(
            (region.lokey, region.hikey, region.pitch_keycenter),
            (12, 12, 11)
        );
    }

    // Comments of both kinds go, a block comment across lines included.
    #[test]
    fn comments_are_skipped() {
        let sfz = read("<region> sample=a.wav /* lokey=1\nhikey=2 */ hivel=90 // lovel=3\n");
        assert_eq!(sfz.problems, []);
        let region = &sfz.regions[0];
        assert_eq!((region.lokey, region.hikey), (0, 127));
        assert_eq!((region.lovel, region.hivel), (1, 90));
    }

    // A bad value is left out and reported where it is; the region keeps the rest.
    #[test]
    fn a_bad_value_is_reported_and_left_out() {
        let sfz = read("<region> sample=a.wav pan=120 lokey=x9 hikey=99\n");
        assert_eq!(sfz.regions[0].pan, 0.0);
        assert_eq!(sfz.regions[0].hikey, 99);
        let texts: Vec<String> = sfz.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "test.sfz:1: pan=120: outside -100 to 100",
                "test.sfz:1: lokey=x9: not a key from 0 to 127 or a note name",
            ]
        );
    }

    // Karoryfer's loop_mode=continuous is loop_continuous.
    #[test]
    fn continuous_is_loop_continuous() {
        let sfz = read("<region> sample=a.wav loop_mode=continuous\n");
        assert_eq!(
            (sfz.regions[0].loop_mode, sfz.problems.len()),
            (Some(LoopMode::Continuous), 0)
        );
    }

    // A Windows-made set's default_path=\samples\ names the folder beside it, not a root.
    #[test]
    fn a_default_path_from_a_backslash_is_beside_the_file() {
        let sfz = read("<control> default_path=\\samples\\\n<region> sample=a.wav\n");
        assert_eq!(sfz.regions[0].sample, "samples/a.wav");
    }

    // A volume past SFZ's -144 to 6 dB plays at the nearer end, reported (libraries write +9
    // and +12; leaving the opcode out played them at 0 dB); one that is no number is left out.
    #[test]
    fn a_volume_past_the_range_plays_at_its_end() {
        let sfz = read(
            "<group> volume=12\n<region> sample=a.wav\n<region> sample=b.wav volume=-200\n\
             <region> sample=c.wav volume=6\n<region> sample=d.wav volume=nan\n",
        );
        let volumes: Vec<f64> = sfz.regions.iter().map(|r| r.volume).collect();
        assert_eq!(volumes, [6.0, -144.0, 6.0, 6.0]);
        let texts: Vec<String> = sfz.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "test.sfz:1: volume=12: outside -144 to 6; it plays at 6",
                "test.sfz:3: volume=-200: outside -144 to 6; it plays at -144",
                "test.sfz:5: volume=nan: not a number",
            ]
        );
    }

    // A region without a sample is reported; end=-1 leaves its region out quietly; an include
    // that cannot be read and one that includes itself are reported.
    #[test]
    fn regions_that_cannot_play_and_includes_that_cannot_be_read() {
        let mut include = |path: &str| match path {
            "loop.sfz" => Ok("#include \"loop.sfz\"\n".to_owned()),
            _ => Err("no such file".to_owned()),
        };
        let sfz = parse(
            "main.sfz",
            "<region> lokey=1\n<region> sample=a.wav end=-1\n#include \"gone.sfz\"\n#include \"loop.sfz\"\n",
            &mut include,
        );
        assert_eq!(sfz.regions, []);
        let texts: Vec<String> = sfz.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "main.sfz:1: the region names no sample; it is left out",
                "main.sfz:3: #include \"gone.sfz\": no such file",
                "loop.sfz:1: #include \"loop.sfz\" includes a file being read",
            ]
        );
    }

    /// `#include "<spelling>"` for each of `spellings`, a line each.
    fn including(spellings: &[&str]) -> String {
        spellings
            .iter()
            .map(|path| format!("#include \"{path}\"\n"))
            .collect()
    }

    // A file that includes itself under other spellings is read again under each one that is
    // not open yet: six spellings would be 326 reads, factorial in the spellings (found
    // 2026-10-22: twelve aborted the program). Reads stop at
    // MAX_INCLUDE_READS, with one problem saying so.
    #[test]
    fn include_reads_stop_at_the_budget() {
        let spellings = [
            "x.sfz",
            "./x.sfz",
            "././x.sfz",
            "./././x.sfz",
            "././././x.sfz",
            "./././././x.sfz",
        ];
        let mut reads = 0;
        let sfz = parse("main.sfz", "#include \"x.sfz\"\n", &mut |_| {
            reads += 1;
            Ok(including(&spellings))
        });
        assert!(reads <= MAX_INCLUDE_READS, "{reads} reads");
        let budget: Vec<&Problem> = sfz
            .problems
            .iter()
            .filter(|problem| problem.text.contains("included files"))
            .collect();
        assert_eq!(budget.len(), 1, "{:?}", sfz.problems);
    }

    // Told each file's identity, the reader knows one file under two spellings: x.sfz including
    // itself as ./x.sfz, and the main file as ././main.sfz, is refused as including a file being
    // read, after one read each; its region is read once.
    #[test]
    fn a_file_under_another_spelling_is_the_same_file() {
        let mut reads = 0;
        let main = "#include \"x.sfz\"\n";
        let sfz = parse_files("main.sfz", "/set/main.sfz", main, &mut |path| {
            reads += 1;
            let name = path.trim_start_matches("./");
            Ok(SetFile {
                id: format!("/set/{name}"),
                text: format!(
                    "{}<region> sample=a.wav\n",
                    including(&["./x.sfz", "././main.sfz"])
                ),
            })
        });
        assert_eq!(reads, 3);
        assert_eq!(sfz.regions.len(), 1);
        let texts: Vec<String> = sfz.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "x.sfz:1: #include \"./x.sfz\" includes a file being read",
                "x.sfz:2: #include \"././main.sfz\" includes a file being read",
            ]
        );
    }

    // Distinct files multiply too: each of four levels including the next ten times would be
    // 11,110 reads. The budget holds them as it holds one file's spellings.
    #[test]
    fn distinct_files_including_each_other_stop_at_the_budget() {
        let mut reads = 0;
        let sfz = parse("main.sfz", &including(&["f1.sfz"; 10]), &mut |path| {
            reads += 1;
            let level: u32 = path[1..path.len() - 4].parse().expect("a level");
            Ok(if level < 4 {
                including(&[format!("f{}.sfz", level + 1).as_str(); 10])
            } else {
                String::new()
            })
        });
        assert!(reads <= MAX_INCLUDE_READS, "{reads} reads");
        assert!(!sfz.problems.is_empty());
    }

    // The text of all the set's files together stops at MAX_SET_BYTES: twenty included files
    // of a megabyte each are not all read, and the first one past the budget is reported.
    #[test]
    fn the_text_read_stops_at_the_byte_budget() {
        let big = format!("// {}\n", "x".repeat(1 << 20));
        let mut bytes = 0;
        let names: Vec<String> = (0..20).map(|n| format!("p{n}.sfz")).collect();
        let main = including(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let sfz = parse("main.sfz", &main, &mut |_| {
            bytes += big.len();
            Ok(big.clone())
        });
        assert!(bytes <= MAX_SET_BYTES + big.len(), "{bytes} bytes given");
        let budget = sfz
            .problems
            .iter()
            .filter(|problem| problem.text.contains("MB"))
            .count();
        assert_eq!(budget, 1, "{:?}", sfz.problems);
    }

    // Problems are listed up to MAX_PROBLEMS, then one line counts the rest, naming where the
    // first of them is.
    #[test]
    fn problems_are_capped_with_a_count_of_the_rest() {
        let text = "junk\n".repeat(MAX_PROBLEMS + 500);
        let sfz = read(&text);
        assert_eq!(sfz.problems.len(), MAX_PROBLEMS + 1);
        let last = sfz.problems.last().expect("the count");
        assert_eq!(
            last.to_string(),
            format!(
                "test.sfz:{}: and 500 more problems from here on",
                MAX_PROBLEMS + 1
            )
        );
    }

    // A region whose key or velocity range is upside down can never play: it is left out and
    // reported, as every other value that cannot be played is.
    #[test]
    fn an_inverted_range_is_reported_and_its_region_left_out() {
        let sfz = read(
            "<region> sample=a.wav lokey=70 hikey=60\n<region> sample=b.wav lovel=100 hivel=20\n<region> sample=c.wav\n",
        );
        assert_eq!(sfz.regions.len(), 1);
        assert_eq!(sfz.regions[0].sample, "c.wav");
        let texts: Vec<String> = sfz.problems.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "test.sfz:1: hikey=60 is below lokey=70; the region never plays and is left out",
                "test.sfz:2: hivel=20 is below lovel=100; the region never plays and is left out",
            ]
        );
    }
}
