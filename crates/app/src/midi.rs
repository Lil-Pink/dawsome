//! MIDI input: every message a port delivers (or the headless door replays) is parsed here once.
//! Notes go to the player's inbox and play from its next call: on the channel the port is linked
//! to, else on the selected channel. So do the performance controls the player plays there: the
//! sustain pedal (CC 64), pitch bend and the mod wheel (CC 1). Controller moves go to a queue the
//! window drains, which turns them into parameter changes, and the first move after the window
//! last looked wakes it; CC 1 and CC 64 go there too (a link can learn them), and a controller
//! link on one of them wins: the player then never hears it. Nothing on this path allocates per
//! message: a full queue drops the message and counts it. Timestamps are ignored, so a message
//! takes effect at the next audio call, and every MIDI channel of a port plays the same rack
//! channel (omni).
//!
//! The system's input ports (the ALSA sequencer's on Linux, WinMM's on Windows) are listed by
//! name, with no hotplug signal, so the window lists them again on a timer; each port that plays
//! has a connection of its own, whose thread delivers into a [`Router`]. Each router also stamps
//! when its port last sent anything ([`Activity`]), for the Library's lights, and, in a
//! `--midi-monitor` run, keeps what arrived for the window to print.

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

use crossbeam_queue::ArrayQueue;
use dawsome_engine::project::{stable_port_name, ChannelId, Project};
use dawsome_engine::realtime::{LiveControl, LiveNote, MidiInbox};
use iced::futures::task::AtomicWaker;
use iced::futures::Stream;
use midir::{Ignore, MidiInput, MidiInputConnection};
use wmidi::MidiMessage;

/// The client name the app's MIDI connections show under in the system.
const CLIENT_NAME: &str = "Dawsome";

/// The client name of this run: [`CLIENT_NAME`], or on Linux the application id a test run
/// sets (`DAWSOME_APP_ID=dawsome-test`). Why: a test's connections are then told apart from the
/// user's own running app in the ALSA sequencer's list (`aconnect -l`), which a test must never
/// touch. (Windows has no application id, and its MIDI inputs no client names to tell apart.)
fn client_name() -> String {
    #[cfg(target_os = "linux")]
    return client_name_for(std::env::var(crate::desktop::APP_ID_VARIABLE).ok());
    #[cfg(not(target_os = "linux"))]
    return CLIENT_NAME.to_owned();
}

#[cfg(target_os = "linux")]
fn client_name_for(app_id: Option<String>) -> String {
    app_id
        .filter(|id| !id.trim().is_empty() && id != crate::desktop::APP_ID)
        .unwrap_or_else(|| CLIENT_NAME.to_owned())
}

/// How long a port's light stays lit after its last message.
pub const LIGHT: Duration = Duration::from_millis(150);

/// Messages a port's monitor keeps for the window to print; more are dropped.
const MONITOR_CAPACITY: usize = 256;

/// Controller moves that can wait for the window; more are dropped and counted.
const CONTROLLER_CAPACITY: usize = 512;

/// MIDI velocities and controller values run 0 to 127; a velocity is the byte over 128, so the
/// default 100 is the 0.78125 a drawn note has.
pub(crate) const VELOCITY_SCALE: f32 = 128.0;

/// The controllers the player plays itself, unless a link takes them: the mod wheel and the
/// sustain pedal.
pub(crate) const WHEEL_CC: u8 = 1;
pub(crate) const PEDAL_CC: u8 = 64;

/// Pitch bend's center: a message's 14-bit value less this is the bend, −8192 to 8191.
const BEND_CENTER: i16 = 8192;

/// A controller moved: CC `cc` on MIDI channel `channel` (0 to 15, as the status byte counts
/// them) to `value` (0 to 127).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControllerMove {
    pub channel: u8,
    pub cc: u8,
    pub value: u8,
}

/// What a MIDI message is to the app.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Incoming {
    Note(LiveNote),
    Controller(ControllerMove),
    /// Pitch bend, −8192 (all the way down) to 8191 (all the way up), 0 at rest.
    Bend(i16),
}

/// One MIDI message as the app uses it: a note on (velocity byte/128), a note off (a note on at
/// velocity 0 is one), a control change, or pitch bend (its 14-bit value less 8192); `None` for
/// anything else or bytes that are not a whole message.
pub fn parse(bytes: &[u8]) -> Option<Incoming> {
    Some(match MidiMessage::try_from(bytes).ok()? {
        MidiMessage::NoteOn(_, note, velocity) => Incoming::Note(LiveNote::On {
            key: u8::from(note),
            velocity: f32::from(u8::from(velocity)) / VELOCITY_SCALE,
        }),
        MidiMessage::NoteOff(_, note, _) => Incoming::Note(LiveNote::Off {
            key: u8::from(note),
        }),
        MidiMessage::ControlChange(channel, function, value) => {
            Incoming::Controller(ControllerMove {
                channel: channel.index(),
                cc: u8::from(function),
                value: u8::from(value),
            })
        }
        MidiMessage::PitchBendChange(_, bend) => {
            // 14 bits: below 16384, so the u16 fits an i16.
            Incoming::Bend(u16::from(bend) as i16 - BEND_CENTER)
        }
        _ => return None,
    })
}

/// The player's control a controller move is, when it is one the player plays (the mod wheel,
/// the pedal) and `linked` (a [`linked_controls`] mask) does not give it to a link.
fn played(moved: ControllerMove, linked: u32) -> Option<LiveControl> {
    let (shift, control) = match moved.cc {
        WHEEL_CC => (0, LiveControl::Wheel(moved.value)),
        PEDAL_CC => (16, LiveControl::Sustain(moved.value)),
        _ => return None,
    };
    let bit = 1u32 << (shift + u32::from(moved.channel & 0x0F));
    (linked & bit == 0).then_some(control)
}

/// `control` at its rest: the wheel at 0, the pedal up, the bend centered.
fn at_rest(control: LiveControl) -> LiveControl {
    match control {
        LiveControl::Wheel(_) => LiveControl::Wheel(0),
        LiveControl::Sustain(_) => LiveControl::Sustain(0),
        LiveControl::Bend(_) => LiveControl::Bend(0),
    }
}

/// Which of the controllers the player plays a link of `project` takes, as a mask: bit n for
/// CC 1 on MIDI channel n (0 to 15), bit 16 + n for CC 64; a link on any channel takes all 16.
/// What a [`Router`] is told ([`Router::links`]), so its port's thread decides with no lock.
pub fn linked_controls(project: &Project) -> u32 {
    let links = &project.midi_links;
    let mut linked = 0;
    for controller in links.targets().filter_map(|target| links.get(target)) {
        let shift = match controller.cc {
            WHEEL_CC => 0,
            PEDAL_CC => 16,
            _ => continue,
        };
        let channels = controller
            .channel
            .map_or(0xFFFF, |channel| 1u32 << (channel & 0x0F));
        linked |= channels << shift;
    }
    linked
}

/// Where controller moves wait for the window. Any number of threads may push at once; pushing
/// never allocates, and a full queue drops the move and counts it.
#[derive(Debug, Clone)]
pub struct ControllerQueue {
    moves: Arc<ArrayQueue<ControllerMove>>,
    dropped: Arc<AtomicU64>,
    arrived: Arc<Arrived>,
}

/// That moves arrived since the window last looked, and the window's wait to wake when they do.
#[derive(Debug, Default)]
struct Arrived {
    pending: AtomicBool,
    waker: AtomicWaker,
}

impl Default for ControllerQueue {
    fn default() -> Self {
        Self {
            moves: Arc::new(ArrayQueue::new(CONTROLLER_CAPACITY)),
            dropped: Arc::new(AtomicU64::new(0)),
            arrived: Arc::default(),
        }
    }
}

/// A queue is itself and its clones: what keeps the window's wait on it running from one
/// subscription to the next.
impl Hash for ControllerQueue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.moves).hash(state);
    }
}

impl ControllerQueue {
    pub fn push(&self, moved: ControllerMove) {
        if self.moves.push(moved).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        // Only the first move since the window looked wakes it, so a burst of moves costs one
        // wake. Why a wake, not a timer: with a port connected and nothing arriving, the window
        // then sleeps, where a 16 ms timer woke it 60 times a second. The wake hands the
        // window's task to its executor; the move itself is queued already.
        if !self.arrived.pending.swap(true, Ordering::AcqRel) {
            self.arrived.waker.wake();
        }
    }

    /// The window's wait for moves: an item whenever moves arrived since it last yielded one,
    /// and nothing at all while none arrive. The moves themselves wait in the queue.
    pub fn moves(&self) -> impl Stream<Item = ()> + Unpin + Send + 'static {
        let arrived = Arc::clone(&self.arrived);
        iced::futures::stream::poll_fn(move |context| {
            if arrived.pending.swap(false, Ordering::AcqRel) {
                return Poll::Ready(Some(()));
            }
            arrived.waker.register(context.waker());
            // A move that came between the look and the register.
            if arrived.pending.swap(false, Ordering::AcqRel) {
                Poll::Ready(Some(()))
            } else {
                Poll::Pending
            }
        })
    }

    /// The oldest move waiting.
    pub fn pop(&self) -> Option<ControllerMove> {
        self.moves.pop()
    }

    /// Whether no move waits.
    pub fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }

    /// Moves dropped so far because the queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// When the ports last sent anything, for the Library's lights. The MIDI threads stamp it and
/// wake the window once per burst; the window reads the stamps. Stamping takes the clock and
/// two atomic stores: no allocation, no lock.
#[derive(Debug, Clone)]
pub struct Activity {
    /// What the stamps count from.
    epoch: Instant,
    arrived: Arc<Arrived>,
}

impl Default for Activity {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            arrived: Arc::default(),
        }
    }
}

/// An activity is itself and its clones: what keeps the window's wait on it running.
impl Hash for Activity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.arrived).hash(state);
    }
}

impl Activity {
    /// Now, as a stamp: milliseconds since the epoch, plus 1 (0 is "never").
    pub fn now(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis())
            .unwrap_or(u64::MAX - 1)
            .saturating_add(1)
    }

    /// Whether a port last stamped at `stamp` is lit at stamp `now`: within [`LIGHT`] of it.
    pub fn lit(stamp: u64, now: u64) -> bool {
        stamp != 0 && now.saturating_sub(stamp) < LIGHT.as_millis() as u64
    }

    /// A port sent something: stamps `last` and wakes the window, once per burst.
    fn stamp(&self, last: &AtomicU64) {
        last.store(self.now(), Ordering::Relaxed);
        if !self.arrived.pending.swap(true, Ordering::AcqRel) {
            self.arrived.waker.wake();
        }
    }

    /// The window's wait for activity: an item whenever a port sent something since it last
    /// yielded one, and nothing while none do.
    pub fn arrivals(&self) -> impl Stream<Item = ()> + Unpin + Send + 'static {
        let arrived = Arc::clone(&self.arrived);
        iced::futures::stream::poll_fn(move |context| {
            if arrived.pending.swap(false, Ordering::AcqRel) {
                return Poll::Ready(Some(()));
            }
            arrived.waker.register(context.waker());
            if arrived.pending.swap(false, Ordering::AcqRel) {
                Poll::Ready(Some(()))
            } else {
                Poll::Pending
            }
        })
    }
}

/// A message as a port's monitor keeps it: its bytes (the first three) and the channel its
/// router sent a note to then, as the router keeps it (id + 1; 0 the selected channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Heard {
    pub bytes: [u8; 3],
    pub len: u8,
    pub routed: u64,
}

impl Heard {
    /// The bytes it holds.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len).min(3)]
    }

    /// The channel its note went to; `None`: the selected channel.
    pub fn channel(&self) -> Option<ChannelId> {
        self.routed
            .checked_sub(1)
            .and_then(|id| u32::try_from(id).ok())
            .map(ChannelId)
    }
}

/// The channel a MIDI port's notes play: the one `project` links `port` to, else `window`'s
/// (the Library's preview channel while there is one, else the instrument window's channel
/// while it is open), else none: the selected channel. The window's connected ports and the
/// `--midi-events` door both route by it.
pub fn port_channel(project: &Project, port: &str, window: Option<ChannelId>) -> Option<ChannelId> {
    project.channel_for_port(port).or(window)
}

/// Whether the notes `port` plays on `channel` (the one [`port_channel`] gives, else the selected
/// one) go on to the channel's MIDI out: not when echo is off there and the port is linked to the
/// channel or is the MIDI out's own port, as its instrument then hears its own keys already and
/// would play each note twice; yes in every other case (a channel without a MIDI out sends
/// nothing whatever this says).
pub fn port_echo(project: &Project, port: &str, channel: Option<ChannelId>) -> bool {
    let Some((id, out)) = channel.and_then(|id| {
        let out = project.channels.get(&id)?.midi_out.as_ref()?;
        Some((id, out))
    }) else {
        return true;
    };
    let linked = project.channel_for_port(port) == Some(id);
    let own = stable_port_name(port) == stable_port_name(&out.port);
    out.echo || !(linked || own)
}

/// Whether `channel` (the one [`port_channel`] gives, else the selected one) has a MIDI out: only
/// then can its instrument send a port's keys back, so only then does [`Router::deliver`] leave
/// out a note on of a key the port holds.
pub fn has_midi_out(project: &Project, channel: Option<ChannelId>) -> bool {
    channel
        .and_then(|id| project.channels.get(&id))
        .is_some_and(|channel| channel.midi_out.is_some())
}

/// Where one MIDI input delivers: the player's inbox for notes, the window's queue for
/// controller moves. Each port's connection holds one of its own (see [`for_port`]).
///
/// [`for_port`]: Router::for_port
#[derive(Debug, Clone)]
pub struct Router {
    notes: MidiInbox,
    controllers: ControllerQueue,
    /// The channel the notes play, as its id + 1; 0 plays the selected channel. The window sets
    /// it from the document's links ([`route`](Router::route)), and the MIDI thread reads it
    /// as each note arrives.
    channel: Arc<AtomicU64>,
    /// Whether the notes go on to their channel's MIDI out ([`port_echo`]), set with the channel.
    out: Arc<AtomicBool>,
    /// Whether the channel has a MIDI out ([`has_midi_out`]), set with the channel: the feedback
    /// guard in [`deliver`](Router::deliver) holds only then. On until told.
    guarded: Arc<AtomicBool>,
    /// The keys this router's port holds (a bit each), for the feedback guard in
    /// [`deliver`](Router::deliver). Only the port's own thread changes them.
    held: Arc<[AtomicU64; 2]>,
    /// The controllers the player plays that the document links to a parameter
    /// ([`linked_controls`]), set with the channel: those move their parameter alone.
    linked: Arc<AtomicU32>,
    /// What this router last handed the player of the mod wheel and of the pedal: what a link
    /// taking either sets back to rest.
    played: Arc<[AtomicU8; 2]>,
    /// The clock and the window's wake for the ports' lights.
    activity: Activity,
    /// When this router's port last sent anything ([`Activity::now`]'s stamps).
    last: Arc<AtomicU64>,
    /// What arrived, kept for the window to print in a `--midi-monitor` run.
    monitor: Option<Arc<ArrayQueue<Heard>>>,
}

impl Router {
    /// A router whose notes play the selected channel.
    pub fn new(notes: MidiInbox, controllers: ControllerQueue) -> Self {
        Self {
            notes,
            controllers,
            channel: Arc::new(AtomicU64::new(0)),
            out: Arc::new(AtomicBool::new(true)),
            guarded: Arc::new(AtomicBool::new(true)),
            held: Arc::default(),
            linked: Arc::default(),
            played: Arc::default(),
            activity: Activity::default(),
            last: Arc::new(AtomicU64::new(0)),
            monitor: None,
        }
    }

    /// The same router, its ports stamping `activity`'s clock and waking its window.
    pub fn with_activity(self, activity: Activity) -> Self {
        Self { activity, ..self }
    }

    /// A router into the same inbox and queue with a channel of its own to route to, keys of
    /// its own (its note-offs end only its notes) and a light of its own: one port's. With
    /// `monitor`, it keeps what arrives for the window to print.
    pub fn for_port(&self, monitor: bool) -> Self {
        Self {
            notes: self.notes.for_port(),
            controllers: self.controllers.clone(),
            channel: Arc::new(AtomicU64::new(0)),
            out: Arc::new(AtomicBool::new(true)),
            guarded: Arc::new(AtomicBool::new(true)),
            held: Arc::default(),
            linked: Arc::default(),
            played: Arc::default(),
            activity: self.activity.clone(),
            last: Arc::new(AtomicU64::new(0)),
            monitor: monitor.then(|| Arc::new(ArrayQueue::new(MONITOR_CAPACITY))),
        }
    }

    /// When its port last sent anything: an [`Activity::now`] stamp, 0 for never.
    pub fn last_heard(&self) -> u64 {
        self.last.load(Ordering::Relaxed)
    }

    /// The messages its monitor kept since the last call, oldest first.
    pub fn heard(&self) -> Vec<Heard> {
        let Some(monitor) = &self.monitor else {
            return Vec::new();
        };
        std::iter::from_fn(|| monitor.pop()).collect()
    }

    /// Lets go of every key this router's port holds, after its last notes: its connection
    /// closed, so their note-offs cannot come. `false` when the inbox had no room for that.
    pub fn release(&self) -> bool {
        self.notes.release()
    }

    /// From now on the notes play `channel`, or the selected channel when `None`.
    pub fn route(&self, channel: Option<ChannelId>) {
        let routed = channel.map_or(0, |id| u64::from(id.0) + 1);
        self.channel.store(routed, Ordering::Relaxed);
    }

    /// From now on the notes go on to their channel's MIDI out when `out`, else they play
    /// quietly: heard and recorded, never sent ([`port_echo`] says which).
    pub fn echo(&self, out: bool) {
        self.out.store(out, Ordering::Relaxed);
    }

    /// From now on a note on of a key the port holds is left out only when `guarded`: the
    /// notes' channel has a MIDI out ([`has_midi_out`]), whose instrument could be sending them
    /// back. Elsewhere it plays, as any repeated key does.
    pub fn guard(&self, guarded: bool) {
        self.guarded.store(guarded, Ordering::Relaxed);
    }

    /// From now on the controllers the player plays that `linked` names ([`linked_controls`])
    /// move only the parameters linked to them. A controller a link newly takes is set back to
    /// rest in the player, when this router left it off rest there.
    ///
    /// Why: a link is learned by moving the controller, which the player hears until the link
    /// is made; left there, the wheel's vibrato would swing every note after, or the pedal hold
    /// every note for good, with nothing left to move them back.
    pub fn links(&self, linked: u32) {
        let before = self.linked.swap(linked, Ordering::SeqCst);
        let taken = linked & !before;
        let controls = [(0, LiveControl::Wheel(0)), (16, LiveControl::Sustain(0))];
        for (index, (shift, rest)) in controls.into_iter().enumerate() {
            if (taken >> shift) & 0xFFFF == 0 {
                continue;
            }
            if self.played[index].swap(0, Ordering::SeqCst) != 0 {
                let routed = self.channel.load(Ordering::Relaxed);
                let channel = routed
                    .checked_sub(1)
                    .and_then(|id| u32::try_from(id).ok())
                    .map(ChannelId);
                let out = self.out.load(Ordering::Relaxed);
                let _ = self.notes.push_control(channel, rest, out);
            }
        }
    }

    /// Marks `key` held or let go by this router's port; whether it was held before.
    fn hold(&self, key: u8, held: bool) -> bool {
        let (word, bit) = (&self.held[usize::from(key >> 6) & 1], 1u64 << (key & 63));
        let before = if held {
            word.fetch_or(bit, Ordering::Relaxed)
        } else {
            word.fetch_and(!bit, Ordering::Relaxed)
        };
        before & bit != 0
    }

    /// Hands the message in `bytes` on, or ignores it; any message lights the port.
    pub fn deliver(&self, bytes: &[u8]) {
        self.deliver_with(bytes, || {});
    }

    /// As [`deliver`](Self::deliver), running `between` after a controller the player plays is
    /// found unlinked and before it is handed in: where another thread's [`links`](Self::links)
    /// may land (a test holds it there).
    fn deliver_with(&self, bytes: &[u8], between: impl FnOnce()) {
        if bytes.is_empty() {
            return;
        }
        self.activity.stamp(&self.last);
        let routed = self.channel.load(Ordering::Relaxed);
        if let Some(monitor) = &self.monitor {
            let mut kept = [0; 3];
            let len = bytes.len().min(3);
            kept[..len].copy_from_slice(&bytes[..len]);
            // A full monitor drops: it is a check's printout, not the music.
            let _ = monitor.push(Heard {
                bytes: kept,
                len: len as u8,
                routed,
            });
        }
        let channel = routed
            .checked_sub(1)
            .and_then(|id| u32::try_from(id).ok())
            .map(ChannelId);
        let out = self.out.load(Ordering::Relaxed);
        match parse(bytes) {
            // The one place a note's way on is decided, note by note: the door the poly bridge's
            // per-note rules grow in.
            Some(Incoming::Note(note)) => {
                let was_held = match note {
                    LiveNote::On { key, .. } => self.hold(key, true),
                    LiveNote::Off { key } => self.hold(key, false),
                };
                let guarded = self.guarded.load(Ordering::Relaxed);
                if guarded && out && was_held && matches!(note, LiveNote::On { .. }) {
                    // A note on for a key the port holds, going out to a MIDI out: the
                    // instrument sending back what it was sent. Played again it would loop.
                    return;
                }
                // A full inbox counts the note itself.
                let _ = match (out, channel) {
                    (true, Some(id)) => self.notes.push_to(id, note),
                    (true, None) => self.notes.push(note),
                    (false, channel) => self.notes.push_quiet(channel, note),
                };
            }
            // A link can learn it, and moves what it is linked to; the wheel and the pedal play
            // the channel too, unless a link has them. A full inbox counts the control itself.
            Some(Incoming::Controller(moved)) => {
                self.controllers.push(moved);
                if let Some(control) = played(moved, self.linked.load(Ordering::SeqCst)) {
                    between();
                    let index = usize::from(moved.cc == PEDAL_CC);
                    let _ = self.notes.push_control(channel, control, out);
                    self.played[index].store(moved.value, Ordering::SeqCst);
                    // A link made since the mask was read (the window learning it from this very
                    // move) took the control while it was on its way: rest it, as `links` would
                    // have had it seen this value. Handed in, then stored, then the mask read
                    // again, all in one order with `links`' own: whichever of the two sees the
                    // other's work sends the rest after the move.
                    if played(moved, self.linked.load(Ordering::SeqCst)).is_none()
                        && self.played[index].swap(0, Ordering::SeqCst) != 0
                    {
                        let _ = self.notes.push_control(channel, at_rest(control), out);
                    }
                }
            }
            Some(Incoming::Bend(bend)) => {
                let _ = self
                    .notes
                    .push_control(channel, LiveControl::Bend(bend), out);
            }
            None => {}
        }
    }

    /// Messages dropped so far because the inbox or the queue was full.
    pub fn dropped(&self) -> u64 {
        self.notes.dropped() + self.controllers.dropped()
    }
}

/// Where the MIDI input ports come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortSource {
    /// The system's ports: listed, and the ones that play connected. Only the window of a run
    /// with nobody scripting it reaches them.
    Devices,
    /// These names stand in for the ports and nothing is ever connected: what a scripted run
    /// shows (`--midi-ports`), and what tests see.
    Given(Vec<String>),
    /// The system's ports whose names start with this, and no other: a `--midi-monitor` run's,
    /// which must never open a port it was not named (the user's keyboard).
    Only(String),
}

/// One port's open connection.
struct Connection {
    name: String,
    /// What the connection's thread delivers to, routing the port's notes.
    router: Router,
    /// Delivers on its own thread for as long as it is held.
    open: MidiInputConnection<()>,
}

impl Connection {
    /// Closes the connection, which stops its thread, then lets go of every key the port holds,
    /// behind its last notes, so none sounds on. Returns whether the inbox had room for that.
    fn close(self) -> bool {
        // Dropping the connection joins its thread: nothing of the port arrives after this.
        drop(self.open);
        self.router.release()
    }
}

/// The MIDI input ports as last listed, and the connections open to the ones that play.
pub struct MidiPorts {
    source: PortSource,
    names: Vec<String>,
    connections: Vec<Connection>,
    /// Why the ports could not be listed or one could not be connected, until the next try
    /// works.
    problem: Option<String>,
    /// Each connection keeps what arrives for the window to print (`--midi-monitor`).
    monitor: bool,
}

impl std::fmt::Debug for MidiPorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let connected: Vec<&str> = self.connections.iter().map(|c| c.name.as_str()).collect();
        f.debug_struct("MidiPorts")
            .field("source", &self.source)
            .field("names", &self.names)
            .field("connected", &connected)
            .finish()
    }
}

impl MidiPorts {
    /// The ports `source` gives, listed once; nothing connected yet.
    pub fn new(source: PortSource) -> Self {
        let mut ports = Self {
            names: Vec::new(),
            source,
            connections: Vec::new(),
            problem: None,
            monitor: false,
        };
        ports.list();
        ports
    }

    /// From now on each connection keeps what arrives for the window to print: a
    /// `--midi-monitor` run, before it connects anything.
    pub fn monitor(&mut self) {
        self.monitor = true;
    }

    /// When each connected port last sent anything ([`Activity::now`] stamps), by name.
    pub fn last_heard(&self) -> Vec<(&str, u64)> {
        self.connections
            .iter()
            .map(|connection| (connection.name.as_str(), connection.router.last_heard()))
            .collect()
    }

    /// What each connected port's monitor kept since the last call, by port name.
    pub fn heard(&self) -> Vec<(&str, Heard)> {
        self.connections
            .iter()
            .flat_map(|connection| {
                let name = connection.name.as_str();
                connection
                    .router
                    .heard()
                    .into_iter()
                    .map(move |heard| (name, heard))
            })
            .collect()
    }

    /// The ports' names, as last listed.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// Whether the system's ports are listed (rather than names given).
    pub fn lists_devices(&self) -> bool {
        matches!(self.source, PortSource::Devices | PortSource::Only(_))
    }

    /// The names of the ports connected now.
    pub fn connected_names(&self) -> Vec<&str> {
        self.connections
            .iter()
            .map(|connection| connection.name.as_str())
            .collect()
    }

    /// Whether any port is connected.
    pub fn connected(&self) -> bool {
        !self.connections.is_empty()
    }

    /// What went wrong at the last listing or connection, if anything.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    /// Lists the ports again, forgetting the last problem when that works. Returns whether
    /// the list changed.
    pub fn list(&mut self) -> bool {
        let names = match &self.source {
            PortSource::Given(names) => names.clone(),
            PortSource::Devices | PortSource::Only(_) => match device_names() {
                Ok(mut names) => {
                    self.problem = None;
                    if let PortSource::Only(prefix) = &self.source {
                        names.retain(|name| name.starts_with(prefix.as_str()));
                    }
                    names
                }
                Err(error) => {
                    self.problem = Some(error);
                    Vec::new()
                }
            },
        };
        let names: Vec<String> = names
            .into_iter()
            .filter(|name| !hidden_port(name))
            .collect();
        let changed = names != self.names;
        self.names = names;
        changed
    }

    /// Connects the listed ports that `plays` and are not connected yet, delivering to
    /// `router`, and closes the connections to ports that no longer play or are gone, each
    /// letting go of the keys its port holds. Returns whether that could not be queued for
    /// some port (the inbox was full), so every live note must end. Given names never connect.
    pub fn connect(&mut self, plays: impl Fn(&str) -> bool, router: &Router) -> bool {
        if !self.lists_devices() {
            return false;
        }
        let names = &self.names;
        // A connection to a device that went (and maybe came back) is closed and, when it is
        // listed and plays, opened again below.
        let dead = dead_connections(self.connections.iter().map(|c| c.name.clone()));
        let (kept, closed): (Vec<Connection>, Vec<Connection>) =
            std::mem::take(&mut self.connections)
                .into_iter()
                .partition(|connection| {
                    names.contains(&connection.name)
                        && plays(&connection.name)
                        && !dead.contains(&connection.name)
                });
        self.connections = kept;
        let mut unreleased = false;
        for connection in closed {
            unreleased |= !connection.close();
        }
        let wanted: Vec<String> = self
            .names
            .iter()
            .filter(|name| plays(name))
            .filter(|name| !self.connections.iter().any(|c| &c.name == *name))
            .cloned()
            .collect();
        for name in wanted {
            let router = router.for_port(self.monitor);
            match open(&name, router.clone()) {
                Ok(open) => self.connections.push(Connection { name, router, open }),
                Err(error) => self.problem = Some(error),
            }
        }
        unreleased
    }

    /// Routes each connected port's notes to the channel `route_of` names for it (or to the
    /// selected channel), on to its MIDI out or not, and guarded or not ([`Router::guard`]), as
    /// `route_of` says; the controllers `linked` names ([`linked_controls`]) move their links
    /// alone.
    pub fn route(&self, linked: u32, route_of: impl Fn(&str) -> (Option<ChannelId>, bool, bool)) {
        for connection in &self.connections {
            let (channel, out, guarded) = route_of(&connection.name);
            connection.router.route(channel);
            connection.router.echo(out);
            connection.router.guard(guarded);
            connection.router.links(linked);
        }
    }

    /// Closes every connection and opens them again to `router`: a new document's player has
    /// an inbox of its own. The old player goes, so the keys held in it need no letting go.
    pub fn reconnect(&mut self, plays: impl Fn(&str) -> bool, router: &Router) {
        self.connections.clear();
        self.connect(plays, router);
    }
}

/// The sequencer address a system port's name ends with ("Oriole:Oriole MIDI 1 28:0" gives
/// "28:0"), or `None` for a name without one. Linux's alone (and the tests'): only its sequencer
/// numbers ports.
#[cfg(any(target_os = "linux", test))]
fn port_address(name: &str) -> Option<&str> {
    let number = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    name.rsplit(' ').next().filter(|last| {
        last.split_once(':')
            .is_some_and(|(client, port)| number(client) && number(port))
    })
}

/// The device addresses delivering to the ports of the sequencer clients named `client`, read
/// from the text of `/proc/asound/seq/clients` ("Connected From: 14:0, 28:0" under each of our
/// ports). Why: a device unplugged and plugged back between two listings comes back under the
/// same name, so the connection is kept, while the kernel dropped its subscription when the
/// device went; the connection then delivers nothing for good (a real controller, 2026-10-24).
fn alive_sources(clients: &str, client: &str) -> std::collections::BTreeSet<String> {
    subscribed(clients, client, "Connected From:")
}

/// The device addresses the ports of the sequencer clients named `client` send to ("Connecting
/// To: 28:0" under each of our ports, a time stamp's "[r:0]" left off), read as
/// [`alive_sources`] reads the other way: the same unplug and plug back leaves a MIDI out's
/// connection sending to nothing, with no error (review 2026-10-25).
fn alive_destinations(clients: &str, client: &str) -> std::collections::BTreeSet<String> {
    subscribed(clients, client, "Connecting To:")
}

/// The addresses listed after `heading` under the ports of the clients named `client`.
fn subscribed(clients: &str, client: &str, heading: &str) -> std::collections::BTreeSet<String> {
    let mut ours = false;
    let mut alive = std::collections::BTreeSet::new();
    for line in clients.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Client") {
            ours = rest.split('"').nth(1) == Some(client);
        } else if ours {
            if let Some(list) = line.strip_prefix(heading) {
                alive.extend(
                    list.split(',')
                        .map(|address| address.split('[').next().unwrap_or("").trim().to_owned())
                        .filter(|address| !address.is_empty()),
                );
            }
        }
    }
    alive
}

/// Of `connected`, the port names whose device no longer delivers to this client: those with
/// an address that `alive` lacks. A name without an address is never counted dead. Linux's
/// alone (and the tests'), as [`port_address`] is.
#[cfg(any(target_os = "linux", test))]
fn dead_names<'a>(
    connected: impl IntoIterator<Item = &'a str>,
    alive: &std::collections::BTreeSet<String>,
) -> Vec<String> {
    connected
        .into_iter()
        .filter(|name| port_address(name).is_some_and(|address| !alive.contains(address)))
        .map(str::to_owned)
        .collect()
}

/// The names of this run's connections whose device went and came back (or went) since they
/// opened: on Linux from the sequencer's client list; elsewhere none.
fn dead_connections(connected: impl IntoIterator<Item = String>) -> Vec<String> {
    dead_by(connected, alive_sources)
}

/// Of `connected`, the full names (sequencer address included) of the output ports this run's
/// MIDI outs hold connections to, those whose device no longer takes from this client: on Linux
/// from the sequencer's client list; elsewhere none.
pub(crate) fn dead_outputs(connected: impl IntoIterator<Item = String>) -> Vec<String> {
    dead_by(connected, alive_destinations)
}

/// Of `connected`, the names whose address `alive` (read from the sequencer's client list for
/// this run's client name) lacks; none off Linux.
fn dead_by(
    connected: impl IntoIterator<Item = String>,
    alive: fn(&str, &str) -> std::collections::BTreeSet<String>,
) -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        let Ok(clients) = std::fs::read_to_string("/proc/asound/seq/clients") else {
            return Vec::new();
        };
        let alive = alive(&clients, &client_name());
        let connected: Vec<String> = connected.into_iter().collect();
        dead_names(connected.iter().map(String::as_str), &alive)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (connected, alive);
        Vec::new()
    }
}

/// The ALSA client name (or WinMM's) this run's connections show under.
pub fn this_client() -> String {
    client_name()
}

/// Whether the port `name` is left out of the port lists: one of this program's own (its MIDI
/// out shows as an input to it, its inputs as outputs), or one of an anonymous RtMidi client
/// ("RtMidiOut Client:RtMidi output 128:0"). Why: playing one's own output loops every note
/// back into itself.
pub fn hidden_port(name: &str) -> bool {
    hidden_port_for(name, &client_name())
}

fn hidden_port_for(name: &str, client: &str) -> bool {
    name.split_once(':')
        .is_some_and(|(owner, _)| owner == client || owner.starts_with("RtMidi"))
}

/// The names of the system's MIDI input ports.
fn device_names() -> Result<Vec<String>, String> {
    let input =
        MidiInput::new(&client_name()).map_err(|error| format!("MIDI input is off: {error}"))?;
    Ok(input
        .ports()
        .iter()
        .filter_map(|port| input.port_name(port).ok())
        .collect())
}

/// A connection to the system's port `name` whose messages go to `router`.
fn open(name: &str, router: Router) -> Result<MidiInputConnection<()>, String> {
    let failed = |error: String| format!("Cannot open MIDI input {name}: {error}");
    let client = client_name();
    let mut input = MidiInput::new(&client).map_err(|error| failed(error.to_string()))?;
    // System exclusive, clock and active sensing play nothing here.
    input.ignore(Ignore::All);
    let port = input
        .ports()
        .into_iter()
        .find(|port| input.port_name(port).is_ok_and(|named| named == name))
        .ok_or_else(|| failed("it is gone".to_owned()))?;
    input
        .connect(&port, &client, move |_, bytes, _| router.deliver(bytes), ())
        .map_err(|error| failed(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::task::{Context, Poll, Wake, Waker};

    use iced::futures::StreamExt;

    use super::*;

    // A real controller unplugged and plugged back between two listings (2026-10-24): the
    // kernel's client list showed client 129 with no "Connected From" while the connection
    // object lived on. This is that list, cut to the lines that matter.
    const CLIENTS: &str = "Client info\n  cur  clients : 7\n\nClient   0 : \"System\" [Kernel Legacy]\n  Port   0 : \"Timer\" (Rwe-) [In/Out]\n    Connecting To: 142:0\nClient  14 : \"Midi Through\" [Kernel Legacy]\n  Port   0 : \"Midi Through Port-0\" (RWe-) [In/Out]\n    Connecting To: 128:0\nClient  28 : \"Oriole\" [Kernel Legacy]\n  Port   0 : \"Oriole MIDI 1\" (RWeX) [In/Out]\nClient 128 : \"Dawsome\" [User Legacy]\n  Port   0 : \"Dawsome\" (-We-) [Out]\n    Connected From: 14:0\n  Output pool :\n    Pool size          : 500\nClient 129 : \"Dawsome\" [User Legacy]\n  Port   0 : \"Dawsome\" (-We-) [Out]\n  Input pool :\n    Peak cells in use  : 1\nClient 142 : \"PipeWire-System\" [User Legacy]\n  Port   0 : \"input\" (rwe-) [In/Out]\n    Connected From: 0:1, 0:0\n";

    #[test]
    fn a_port_name_ends_with_its_sequencer_address() {
        assert_eq!(port_address("Oriole:Oriole MIDI 1 28:0"), Some("28:0"));
        assert_eq!(
            port_address("Midi Through:Midi Through Port-0 14:0"),
            Some("14:0")
        );
        assert_eq!(port_address("Port A"), None);
        assert_eq!(port_address("Odd name 12:x"), None);
    }

    #[test]
    fn the_clients_list_names_the_devices_still_delivering_to_this_client() {
        let alive = alive_sources(CLIENTS, "Dawsome");
        assert_eq!(
            alive.iter().collect::<Vec<_>>(),
            ["14:0"],
            "PipeWire's sources are not ours"
        );
        assert!(alive_sources(CLIENTS, "dawsome-test").is_empty());
    }

    // A MIDI out's connection as the kernel listed it (2026-10-25, a test's own virtual port as
    // the device): under our client "Connecting To:" with the subscription's time stamp. Once
    // the device goes, the line goes, and the connection's port name is dead.
    #[test]
    fn the_clients_list_names_the_devices_this_client_still_sends_to() {
        let sending = "Client 128 : \"Test\" [User Legacy]\n  Port   0 : \"Test\" (-We-) [Out]\n    Connected From: 129:0[r:0]\nClient 129 : \"Dawsome\" [User Legacy]\n  Port   0 : \"Dawsome\" (R-e-) [In]\n    Connecting To: 128:0[r:0], 28:0\nClient 142 : \"PipeWire-System\" [User Legacy]\n  Port   0 : \"input\" (rwe-) [In/Out]\n    Connecting To: 14:0\n";
        let alive = alive_destinations(sending, "Dawsome");
        assert_eq!(alive.iter().collect::<Vec<_>>(), ["128:0", "28:0"]);
        assert!(alive_sources(sending, "Dawsome").is_empty());
        assert!(dead_names(["Test:Test 128:0"], &alive).is_empty());
        let gone = alive_destinations(CLIENTS, "Dawsome");
        assert_eq!(dead_names(["Test:Test 128:0"], &gone), ["Test:Test 128:0"]);
    }

    #[test]
    fn a_connection_whose_device_stopped_delivering_is_dead() {
        let alive = alive_sources(CLIENTS, "Dawsome");
        let connected = [
            "Midi Through:Midi Through Port-0 14:0",
            "Oriole:Oriole MIDI 1 28:0",
            "Port A",
        ];
        assert_eq!(dead_names(connected, &alive), ["Oriole:Oriole MIDI 1 28:0"]);
    }

    /// A waker that counts how often it is woken.
    struct Counter(AtomicUsize);

    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    // The window waits on the queue's moves instead of polling it: with nothing arriving it
    // is never woken, and a burst of moves wakes it once, however many there are, until it
    // has taken them.
    #[test]
    fn the_window_is_woken_by_moves_and_only_by_moves() {
        let queue = ControllerQueue::default();
        let mut moves = queue.moves();
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&counter));
        let mut context = Context::from_waker(&waker);
        let woken = || counter.0.load(Ordering::Relaxed);
        assert_eq!(moves.poll_next_unpin(&mut context), Poll::Pending);
        assert_eq!(woken(), 0, "nothing arrived");
        let moved = ControllerMove {
            channel: 0,
            cc: 7,
            value: 64,
        };
        for _ in 0..3 {
            queue.push(moved);
        }
        assert_eq!(woken(), 1, "one wake for the burst");
        assert_eq!(moves.poll_next_unpin(&mut context), Poll::Ready(Some(())));
        assert_eq!(moves.poll_next_unpin(&mut context), Poll::Pending);
        assert_eq!(queue.pop(), Some(moved), "the moves wait for the window");
        queue.push(moved);
        assert_eq!(woken(), 2);
        assert_eq!(moves.poll_next_unpin(&mut context), Poll::Ready(Some(())));
    }

    // By hand from the MIDI 1.0 status bytes: 0x9n note on, 0x8n note off, 0xBn control
    // change, 0xEn pitch bend, n the channel; velocity v plays at v/128.
    #[test]
    fn notes_and_controllers_parse_and_everything_else_is_ignored() {
        assert_eq!(
            parse(&[0x90, 60, 100]),
            Some(Incoming::Note(LiveNote::On {
                key: 60,
                velocity: 0.78125
            }))
        );
        assert_eq!(
            parse(&[0x9F, 64, 127]),
            Some(Incoming::Note(LiveNote::On {
                key: 64,
                velocity: 0.992_187_5
            })),
            "any channel plays: omni"
        );
        assert_eq!(
            parse(&[0x90, 64, 0]),
            Some(Incoming::Note(LiveNote::Off { key: 64 })),
            "a note on at velocity 0 is a note off"
        );
        assert_eq!(
            parse(&[0x80, 67, 64]),
            Some(Incoming::Note(LiveNote::Off { key: 67 }))
        );
        assert_eq!(
            parse(&[0xB1, 10, 0]),
            Some(Incoming::Controller(ControllerMove {
                channel: 1,
                cc: 10,
                value: 0
            }))
        );
        // En ll mm: ll + 128 mm, less 8192.
        assert_eq!(parse(&[0xE0, 0x00, 0x40]), Some(Incoming::Bend(0)));
        assert_eq!(parse(&[0xE3, 0x7F, 0x7F]), Some(Incoming::Bend(8191)));
        assert_eq!(parse(&[0xE0, 0x00, 0x00]), Some(Incoming::Bend(-8192)));
        assert_eq!(parse(&[0xE0, 0x01, 0x40]), Some(Incoming::Bend(1)));
        for ignored in [
            &[0xC0, 5][..],
            &[0xE0, 0, 0x80],
            &[0x90, 60],
            &[60, 100],
            &[],
            &[0x90, 200, 100],
        ] {
            assert_eq!(parse(ignored), None, "{ignored:02X?}");
        }
    }

    // By hand: a link on CC 1 on any channel takes bits 0..15, one on CC 64 on MIDI channel 3
    // bit 16 + 2; other controllers take none. A taken controller is the link's alone; the
    // others are the player's wheel and pedal on their channel.
    #[test]
    fn links_on_the_wheel_or_the_pedal_take_them_from_the_player() {
        use dawsome_engine::project::{AutomationTarget, Controller, InsertId};
        let mut project = crate::project_file::new_document();
        assert_eq!(linked_controls(&project), 0);
        let link = |cc, channel| Controller { cc, channel };
        project.midi_links.insert(
            AutomationTarget::InsertFader(InsertId(1)),
            link(WHEEL_CC, None),
        );
        project.midi_links.insert(
            AutomationTarget::InsertPan(InsertId(1)),
            link(PEDAL_CC, Some(2)),
        );
        project
            .midi_links
            .insert(AutomationTarget::InsertFader(InsertId(2)), link(7, None));
        let linked = linked_controls(&project);
        assert_eq!(linked, 0xFFFF | 1 << 18);
        let moved = |cc, channel, value| ControllerMove { cc, channel, value };
        assert_eq!(played(moved(1, 5, 9), linked), None, "the wheel is linked");
        assert_eq!(
            played(moved(64, 0, 127), linked),
            Some(LiveControl::Sustain(127))
        );
        assert_eq!(played(moved(64, 2, 127), linked), None, "on its channel");
        assert_eq!(played(moved(1, 5, 9), 0), Some(LiveControl::Wheel(9)));
        assert_eq!(played(moved(7, 0, 9), 0), None, "the player plays no CC 7");
    }

    // A port's pedal, bend and wheel reach the player on the channel the port plays, and still
    // reach the window's queue (a link can learn them); a linked pedal reaches the queue alone.
    #[test]
    fn a_ports_controls_reach_the_player_and_the_queue() {
        use dawsome_engine::realtime::EngineEvent;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p10_link.json");
        let opened = crate::project_file::open(&path).expect("p10_link opens");
        let (mut session, mut player) =
            crate::session::Session::open(opened.project, opened.samples_dir, false)
                .expect("opens");
        let queue = ControllerQueue::default();
        let port = Router::new(session.midi_inbox(), queue.clone()).for_port(false);
        port.route(Some(ChannelId(2)));
        for bytes in [
            &[0xB0, 64, 127][..],
            &[0xE0, 0x7F, 0x7F],
            &[0xB0, 1, 30],
            &[0x90, 60, 64],
            &[0x80, 60, 0],
        ] {
            port.deliver(bytes);
        }
        player.process(&mut [[0.0; 2]; 64]);
        let ended = |events: &[EngineEvent]| {
            events
                .iter()
                .any(|event| matches!(event, EngineEvent::LiveNoteEnded { .. }))
        };
        assert!(!ended(&session.take_events()), "the pedal holds the note");
        let moves: Vec<(u8, u8)> = std::iter::from_fn(|| queue.pop())
            .map(|moved| (moved.cc, moved.value))
            .collect();
        assert_eq!(moves, [(64, 127), (1, 30)]);
        // Linked, the pedal reaches the queue alone: pressed, it holds nothing.
        port.links(1 << 16);
        player.process(&mut [[0.0; 2]; 64]);
        let _ = session.take_events();
        port.deliver(&[0xB0, 64, 127]);
        port.deliver(&[0x90, 62, 64]);
        port.deliver(&[0x80, 62, 0]);
        player.process(&mut [[0.0; 2]; 64]);
        assert!(
            ended(&session.take_events()),
            "a linked pedal holds nothing"
        );
        assert_eq!(queue.pop().map(|moved| moved.value), Some(127));
        // Unlinked again, it holds and lifts.
        port.links(0);
        port.deliver(&[0xB0, 64, 127]);
        port.deliver(&[0x90, 64, 64]);
        port.deliver(&[0x80, 64, 0]);
        player.process(&mut [[0.0; 2]; 64]);
        assert!(!ended(&session.take_events()), "held");
        port.deliver(&[0xB0, 64, 0]);
        player.process(&mut [[0.0; 2]; 64]);
        assert!(ended(&session.take_events()), "unlinked, it lifts");
    }

    // A race: the window makes
    // the link (on its thread) just after the port's thread found the pedal unlinked and before
    // it handed the press in. The press still reaches the player, and the link swallows every
    // lift after; the port's thread, reading the mask again once the press is in, rests the
    // pedal itself, so the note let go of next ends.
    #[test]
    fn a_link_made_while_a_press_is_on_its_way_still_lifts_the_pedal() {
        use dawsome_engine::realtime::EngineEvent;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p10_link.json");
        let opened = crate::project_file::open(&path).expect("p10_link opens");
        let (mut session, mut player) =
            crate::session::Session::open(opened.project, opened.samples_dir, false)
                .expect("opens");
        let port = Router::new(session.midi_inbox(), ControllerQueue::default()).for_port(false);
        port.deliver_with(&[0xB0, 64, 127], || port.links(1 << 16));
        port.deliver(&[0x90, 60, 64]);
        port.deliver(&[0x80, 60, 0]);
        player.process(&mut [[0.0; 2]; 64]);
        let ended = session
            .take_events()
            .iter()
            .any(|event| matches!(event, EngineEvent::LiveNoteEnded { .. }));
        assert!(ended, "the pedal was rested: the note let go of ends");
    }

    /// p22_sine with a second channel "Sine B" (2) on the same set, a session of it and its
    /// player.
    fn two_sines() -> (crate::session::Session, dawsome_engine::realtime::Player) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p22_sine.json");
        let mut opened = crate::project_file::open(&path).expect("p22_sine opens");
        let b = dawsome_engine::project::Channel::instrument("Sine B", "p22_sine/sine.sfz");
        opened.project.channels.insert(ChannelId(2), b);
        opened.project.next_channel_id = ChannelId(3);
        crate::session::Session::open(opened.project, opened.samples_dir, false).expect("opens")
    }

    // The link's rest goes to the channel the port plays now, not
    // the one it moved the wheel on. The wheel up on A (1), the port moved to B (2), then CC 1
    // linked: the rest lands on B and rests A too (the engine's one channel a device's wheel is
    // on), so the port's next A4 on A plays as one no wheel ever touched, bit for bit.
    #[test]
    fn a_links_rest_reaches_the_channel_the_wheel_was_left_on() {
        let note_on_a = |wheel: bool| {
            let (session, mut player) = two_sines();
            let port =
                Router::new(session.midi_inbox(), ControllerQueue::default()).for_port(false);
            if wheel {
                port.route(Some(ChannelId(1)));
                port.deliver(&[0xB0, 1, 100]);
                player.process(&mut [[0.0; 2]; 512]);
                port.route(Some(ChannelId(2)));
                port.links(0xFFFF);
                player.process(&mut [[0.0; 2]; 512]);
            }
            port.route(Some(ChannelId(1)));
            port.deliver(&[0x90, 69, 127]);
            let mut out = vec![[0.0f32; 2]; 40 * 512];
            for call in out.chunks_mut(512) {
                player.process(call);
            }
            out
        };
        let (plain, rested) = (note_on_a(false), note_on_a(true));
        assert!(plain.iter().any(|frame| frame[0] != 0.0), "the note sounds");
        assert!(plain == rested, "the wheel left on A was rested");
    }

    // A link learned while the pedal was down takes it from the player: the router sets the
    // pedal it left down back up, so the notes it held end and the next ones are not held; a
    // link made while the pedal is up sends nothing.
    #[test]
    fn a_link_that_takes_the_pedal_lifts_what_the_player_holds() {
        use dawsome_engine::realtime::EngineEvent;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p10_link.json");
        let opened = crate::project_file::open(&path).expect("p10_link opens");
        let (mut session, mut player) =
            crate::session::Session::open(opened.project, opened.samples_dir, false)
                .expect("opens");
        let port = Router::new(session.midi_inbox(), ControllerQueue::default()).for_port(false);
        let ends = |session: &mut crate::session::Session| {
            session
                .take_events()
                .iter()
                .filter(|event| matches!(event, EngineEvent::LiveNoteEnded { .. }))
                .count()
        };
        port.deliver(&[0xB0, 64, 127]);
        port.deliver(&[0x90, 60, 64]);
        port.deliver(&[0x80, 60, 0]);
        player.process(&mut [[0.0; 2]; 64]);
        assert_eq!(ends(&mut session), 0, "held");
        port.links(1 << 16);
        player.process(&mut [[0.0; 2]; 64]);
        assert_eq!(ends(&mut session), 1, "the link lifted it");
        port.deliver(&[0x90, 62, 64]);
        port.deliver(&[0x80, 62, 0]);
        player.process(&mut [[0.0; 2]; 64]);
        assert_eq!(ends(&mut session), 1, "no longer held");
        port.links(0);
        port.links(1 << 16);
        player.process(&mut [[0.0; 2]; 64]);
        assert_eq!(ends(&mut session), 0, "nothing to lift");
    }

    #[test]
    fn a_full_controller_queue_drops_and_counts() {
        let queue = ControllerQueue::default();
        let moved = ControllerMove {
            channel: 0,
            cc: 7,
            value: 1,
        };
        for _ in 0..CONTROLLER_CAPACITY + 3 {
            queue.push(moved);
        }
        assert_eq!(queue.dropped(), 3);
        assert_eq!(
            std::iter::from_fn(|| queue.pop()).count(),
            CONTROLLER_CAPACITY
        );
    }

    // A port's own router plays the channel it is routed to whatever is selected (p10_link:
    // "kick", channel 1, is selected), and the selected channel again once routed to none; the
    // router it was made from still plays the selected channel.
    #[test]
    fn a_routed_ports_notes_play_its_channel() {
        use dawsome_engine::realtime::EngineEvent;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p10_link.json");
        let opened = crate::project_file::open(&path).expect("p10_link opens");
        let (mut session, mut player) =
            crate::session::Session::open(opened.project, opened.samples_dir, false)
                .expect("opens");
        let router = Router::new(session.midi_inbox(), ControllerQueue::default());
        let port = router.for_port(false);
        port.route(Some(ChannelId(2)));
        port.deliver(&[0x90, 60, 64]);
        router.deliver(&[0x90, 62, 64]);
        port.route(None);
        port.deliver(&[0x90, 64, 64]);
        player.process(&mut [[0.0; 2]; 64]);
        let started = |channel, key| EngineEvent::LiveNoteStarted {
            channel: ChannelId(channel),
            key,
            velocity: 0.5,
            tick: None,
            frame: 0,
        };
        assert_eq!(
            session.take_events(),
            [started(2, 60), started(1, 62), started(1, 64)]
        );
    }

    // Each port's router holds keys of its own: two ports on one key and two channels both
    // play, one port's note-off ends its own note only, and a release (what closing a port's
    // connection sends) lets go of that port's keys alone.
    #[test]
    fn a_ports_router_holds_its_own_keys_and_lets_them_go() {
        use dawsome_engine::realtime::EngineEvent;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p10_link.json");
        let opened = crate::project_file::open(&path).expect("p10_link opens");
        let (mut session, mut player) =
            crate::session::Session::open(opened.project, opened.samples_dir, false)
                .expect("opens");
        let router = Router::new(session.midi_inbox(), ControllerQueue::default());
        let (linked, other) = (router.for_port(false), router.for_port(false));
        linked.route(Some(ChannelId(2)));
        linked.deliver(&[0x90, 60, 64]);
        linked.deliver(&[0x90, 62, 64]);
        other.deliver(&[0x90, 60, 64]);
        linked.deliver(&[0x80, 60, 0]);
        player.process(&mut [[0.0; 2]; 64]);
        assert!(linked.release());
        player.process(&mut [[0.0; 2]; 64]);
        // The second call's first frame, after one of 64: the release's.
        let event = |started, channel, key, frame| match started {
            true => EngineEvent::LiveNoteStarted {
                channel: ChannelId(channel),
                key,
                velocity: 0.5,
                tick: None,
                frame,
            },
            false => EngineEvent::LiveNoteEnded {
                channel: ChannelId(channel),
                key,
                tick: None,
                frame,
            },
        };
        assert_eq!(
            session.take_events(),
            [
                event(true, 2, 60, 0),
                event(true, 2, 62, 0),
                event(true, 1, 60, 0),
                event(false, 2, 60, 0),
                event(false, 2, 62, 64),
            ]
        );
    }

    // A port's light and its monitor. Any message stamps the port's router,
    // a note, a controller or one the app ignores alike (an empty one is no message), and the
    // window is woken once for the burst; a stamp is lit for 150 ms after it and not before
    // anything came. A router that monitors keeps each message's bytes and the channel its
    // note went to then (id + 1, 0 the selected channel); one that does not keeps nothing.
    // The client name of a test run is its application id, of any other run "Dawsome".
    #[test]
    fn a_ports_messages_light_it_and_its_monitor_keeps_them() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p10_link.json");
        let opened = crate::project_file::open(&path).expect("p10_link opens");
        let (session, _player) =
            crate::session::Session::open(opened.project, opened.samples_dir, false)
                .expect("opens");
        let activity = Activity::default();
        let mut arrivals = activity.arrivals();
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&counter));
        let mut context = Context::from_waker(&waker);
        assert_eq!(arrivals.poll_next_unpin(&mut context), Poll::Pending);
        let router = Router::new(session.midi_inbox(), ControllerQueue::default())
            .with_activity(activity.clone());
        let (port, quiet) = (router.for_port(true), router.for_port(false));
        assert_eq!(port.last_heard(), 0, "nothing came yet");
        assert!(!Activity::lit(port.last_heard(), activity.now()));
        port.route(Some(ChannelId(2)));
        port.deliver(&[0x90, 60, 64]);
        port.route(None);
        port.deliver(&[0xB0, 7, 100]);
        port.deliver(&[0xE0, 0, 64]);
        port.deliver(&[]);
        quiet.deliver(&[0x90, 62, 64]);
        assert_eq!(
            counter.0.load(Ordering::Relaxed),
            1,
            "one wake for the burst"
        );
        assert_eq!(
            arrivals.poll_next_unpin(&mut context),
            Poll::Ready(Some(()))
        );
        let stamp = port.last_heard();
        assert!(stamp > 0 && quiet.last_heard() > 0);
        assert!(Activity::lit(stamp, stamp + 149));
        assert!(!Activity::lit(stamp, stamp + 150));
        let heard = port.heard();
        let kept: Vec<(&[u8], Option<ChannelId>)> = heard
            .iter()
            .map(|heard| (heard.bytes(), heard.channel()))
            .collect();
        assert_eq!(
            kept,
            [
                (&[0x90, 60, 64][..], Some(ChannelId(2))),
                (&[0xB0, 7, 100][..], None),
                (&[0xE0, 0, 64][..], None),
            ]
        );
        assert!(port.heard().is_empty(), "taken once");
        assert!(quiet.heard().is_empty());
        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                client_name_for(Some("dawsome-test".to_owned())),
                "dawsome-test"
            );
            for not_a_test in [None, Some(String::new()), Some("dawsome".to_owned())] {
                assert_eq!(client_name_for(not_a_test), "Dawsome");
            }
        }
    }

    // This program's own client (its MIDI out shows as an input) and anonymous RtMidi clients
    // are left out of the list, as given names or the system's; nothing else is.
    #[test]
    fn our_own_and_rtmidi_clients_are_not_listed() {
        let own = format!("{}:{} 129:0", this_client(), this_client());
        let names = vec![
            "Oriole:Oriole MIDI 1 28:0".to_owned(),
            "RtMidiOut Client:RtMidi output 128:0".to_owned(),
            own,
            "Midi Through:Midi Through Port-0 14:0".to_owned(),
        ];
        let ports = MidiPorts::new(PortSource::Given(names));
        assert_eq!(
            ports.names(),
            [
                "Oriole:Oriole MIDI 1 28:0",
                "Midi Through:Midi Through Port-0 14:0"
            ]
        );
        assert!(hidden_port_for("Dawsome:Dawsome 130:0", "Dawsome"));
        assert!(!hidden_port_for("Dawsome:Dawsome 130:0", "dawsome-test"));
        assert!(!hidden_port_for("Port A", "Dawsome"));
    }

    /// p19_echo.json: "Synth" (1, no link, echo off), "Linked" (2, linked to its own port, echo
    /// off), "Echo" (3, linked to its own port, echo on).
    fn echo_project() -> Project {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p19_echo.json");
        crate::project_file::open(&path)
            .expect("p19_echo opens")
            .project
    }

    // By the rule of tests/fixtures/gen_p19_expected.py ("echo"): quiet only where echo is off
    // and the port is linked to the channel or is its MIDI out's own port (ALSA numbers or not).
    #[test]
    fn a_port_echoes_unless_its_instrument_hears_its_keys_already() {
        let project = echo_project();
        let (synth, linked, echo) = (Some(ChannelId(1)), Some(ChannelId(2)), Some(ChannelId(3)));
        let cases = [
            ("Keys:Keys MIDI 1 20:0", synth, true),
            ("Synth:Synth MIDI 1 24:0", synth, false),
            ("Synth:Synth MIDI 1", synth, false),
            ("Linked:Linked MIDI 1 28:0", linked, false),
            ("Keys:Keys MIDI 1 20:0", linked, true),
            ("Echo:Echo MIDI 1 32:0", echo, true),
            ("Synth:Synth MIDI 1 24:0", echo, true),
            ("Keys:Keys MIDI 1 20:0", None, true),
            ("Keys:Keys MIDI 1 20:0", Some(ChannelId(9)), true),
        ];
        for (port, channel, echoes) in cases {
            assert_eq!(
                port_echo(&project, port, channel),
                echoes,
                "{port} on {channel:?}"
            );
        }
    }

    // A quiet port's notes still play (the player starts and ends them) but never reach the MIDI
    // out; an echoing port's do, and a note on of a key it holds already (sent back by the
    // instrument) is left out, while a quiet port's goes on to the player as any note does.
    #[test]
    fn a_quiet_port_plays_without_sending_and_an_echoing_one_drops_what_comes_back() {
        use dawsome_engine::realtime::EngineEvent;
        let (mut session, mut player) =
            crate::session::Session::open(echo_project(), std::path::PathBuf::new(), false)
                .expect("opens");
        let mut sent = session.take_midi_out().expect("the ring");
        let router = Router::new(session.midi_inbox(), ControllerQueue::default());
        let (quiet, echoing) = (router.for_port(false), router.for_port(false));
        quiet.route(Some(ChannelId(2)));
        quiet.echo(false);
        echoing.route(Some(ChannelId(3)));
        quiet.deliver(&[0x90, 64, 80]);
        quiet.deliver(&[0x90, 64, 80]);
        echoing.deliver(&[0x90, 67, 70]);
        echoing.deliver(&[0x90, 67, 70]);
        player.process(&mut [[0.0; 2]; 64]);
        let started: Vec<(ChannelId, u8)> = session
            .take_events()
            .into_iter()
            .filter_map(|event| match event {
                EngineEvent::LiveNoteStarted { channel, key, .. } => Some((channel, key)),
                _ => None,
            })
            .collect();
        assert_eq!(
            started,
            [(ChannelId(2), 64), (ChannelId(2), 64), (ChannelId(3), 67)],
            "the quiet note plays (again on its second note on), the echo once"
        );
        let mut out = Vec::new();
        sent.read_into(&mut out);
        let bytes: Vec<&[u8]> = out.iter().map(|event| event.bytes()).collect();
        assert_eq!(bytes, [&[0x92, 67, 70][..]]);
        echoing.deliver(&[0x80, 67, 0]);
        echoing.deliver(&[0x90, 67, 70]);
        player.process(&mut [[0.0; 2]; 64]);
        out.clear();
        sent.read_into(&mut out);
        let bytes: Vec<&[u8]> = out.iter().map(|event| event.bytes()).collect();
        assert_eq!(
            bytes,
            [&[0x82, 67, 0x40][..], &[0x92, 67, 70]],
            "let go, the key plays again"
        );
    }

    // The feedback guard is for an instrument sending back
    // what a MIDI out sent it, so it holds only on a channel with a MIDI out. p19_rack: "hit" (5)
    // is a sampler, where a second note on of a key the port holds plays again; on "Synth A" (1,
    // a MIDI out) it is left out, as before. Routed as the window routes a port.
    #[test]
    fn only_a_channel_with_a_midi_out_leaves_out_a_held_keys_note_on() {
        use dawsome_engine::realtime::EngineEvent;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p19_rack.json");
        let opened = crate::project_file::open(&path).expect("p19_rack opens");
        let project = opened.project.clone();
        let (mut session, mut player) =
            crate::session::Session::open(opened.project, opened.samples_dir, false)
                .expect("opens");
        let router = Router::new(session.midi_inbox(), ControllerQueue::default());
        let keys = "Keys:Keys MIDI 1 20:0";
        for (id, plays) in [(ChannelId(5), 2), (ChannelId(1), 1)] {
            let port = router.for_port(false);
            let channel = Some(id);
            port.route(channel);
            port.echo(port_echo(&project, keys, channel));
            port.guard(has_midi_out(&project, channel));
            port.deliver(&[0x90, 0x3C, 0x50]);
            port.deliver(&[0x90, 0x3C, 0x50]);
            player.process(&mut [[0.0; 2]; 64]);
            let started = session
                .take_events()
                .into_iter()
                .filter(|event| {
                    matches!(event, EngineEvent::LiveNoteStarted { channel, key: 60, .. } if *channel == id)
                })
                .count();
            assert_eq!(started, plays, "channel {id:?}");
        }
        assert!(
            !has_midi_out(&project, None),
            "the selected channel when none is"
        );
    }

    // Given names list as given and never connect: what a scripted run and the tests see.
    #[test]
    fn given_ports_list_their_names_and_never_connect() {
        let names = vec!["Port A".to_owned(), "Port B".to_owned()];
        let mut ports = MidiPorts::new(PortSource::Given(names.clone()));
        assert_eq!(ports.names(), names);
        assert!(!ports.lists_devices());
        let (session, _player) = crate::session::Session::open(
            crate::project_file::new_document(),
            std::path::PathBuf::new(),
            false,
        )
        .expect("opens");
        let router = Router::new(session.midi_inbox(), ControllerQueue::default());
        assert!(!ports.connect(|_| true, &router));
        assert!(!ports.connected());
        assert!(!ports.list(), "the same names again");
    }
}
