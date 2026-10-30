//! MIDI out: the thread that sends what the input channels' MIDI outs send. The player stamps
//! each note on and note off with the output frame it is due on and hands it over its MIDI-out
//! ring ([`MidiOutReader`]); this thread reads the ring, waits until the frame plays on the
//! [`PlaybackClock`] the output stream publishes (a message whose time is unknown or past goes at
//! once and is counted late), and sends it to the port of its channel's route.
//!
//! The window tells the thread each channel's route (port, MIDI channel, program) after every
//! change to the document, and the routes decide what else goes out: a program change when a
//! route's program is set or a document opens, never on Play (that would throw away edits not yet
//! saved on the instrument); all notes off (`Bn 7B 00`, then all sound off `Bn 78 00`) on the old
//! place of a route that goes or moves (a channel deleted, its port or MIDI channel changed, File
//! → New, quit), on every route of a port that takes bytes again after a send to it failed or its
//! device went (unplugged; on Linux read from the sequencer's client list), and
//! on every route when the player lost a note off. Stop needs none: the player sends the note
//! offs of the notes it started, and keys held on the instrument are left alone.
//!
//! Every byte goes through [`Sending::send_checked`], which passes notes, all notes off, program
//! changes and the performance controls a keyboard plays (the sustain pedal, the mod wheel, pitch
//! bend) only.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dawsome_engine::midi_out::{MidiOutEvent, MidiOutReader};
use dawsome_engine::project::{stable_port_name, ChannelId, Project};
use midir::{MidiOutput, MidiOutputConnection};

use crate::audio::PlaybackClock;
use crate::midi;

/// A program change's status nibble; a control change's; pitch bend's.
const PROGRAM: u8 = 0xC0;
const CONTROL: u8 = 0xB0;
const BEND: u8 = 0xE0;

/// The two controllers a MIDI out sends: all notes off ends every note the way a note off would
/// (with the instrument's release), all sound off silences what still rings after that.
const ALL_NOTES_OFF: u8 = 123;
const ALL_SOUND_OFF: u8 = 120;

/// How often the ring is read while a route is set: a message waits at most this long past the
/// time it is due.
const POLL: Duration = Duration::from_millis(1);

/// How long a port that would not open is left before it is tried again.
const RETRY: Duration = Duration::from_secs(1);

/// The messages a recording keeps; later ones are dropped (a scripted run's stand-in for the
/// ports, which no one reads past that).
const RECORDING_CAPACITY: usize = 4096;

/// Where one channel's MIDI out sends: the port by its stable name
/// ([`stable_port_name`]), the MIDI channel as the status byte counts it (0 to 15), and the
/// program the port is set to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub port: String,
    pub channel: u8,
    pub program: Option<u8>,
}

impl Route {
    /// The port and MIDI channel its notes sound on.
    fn place(&self) -> (String, u8) {
        (self.port.clone(), self.channel)
    }
}

/// Each channel's route, by the channel it is the MIDI out of.
pub type Routes = BTreeMap<ChannelId, Route>;

/// The route of every channel of `project` that has a MIDI out.
pub fn routes(project: &Project) -> Routes {
    project
        .channels
        .iter()
        .filter_map(|(&id, channel)| {
            let out = channel.midi_out.as_ref()?;
            let route = Route {
                port: stable_port_name(&out.port).to_owned(),
                channel: out.channel.saturating_sub(1).min(0x0F),
                program: out.program,
            };
            Some((id, route))
        })
        .collect()
}

/// Whether a MIDI out may send `bytes`: a note off or note on (`8n kk vv`, `9n kk vv`), all notes
/// off or all sound off (`Bn 7B 00`, `Bn 78 00`), a program change (`Cn pp`), or what a keyboard's
/// player plays beside its keys and the engine sends on: the mod wheel and the sustain pedal
/// (`Bn 01 vv`, `Bn 40 vv`) and pitch bend (`En ll mm`), each whole. Nothing else, not even another
/// controller or a note of two bytes.
///
/// Why so few: the instrument on the other end (a hardware synthesizer) takes system exclusive
/// messages that can rewrite the program it runs or overwrite the sounds saved on it, with no way back,
/// and controllers that change it in ways no project asked for. The wheel, the pedal and the bend
/// are what its own keyboard would send while played, and change nothing saved. Nothing a MIDI
/// out does needs more than these, so nothing more may pass, whatever made the bytes.
fn allowed(bytes: &[u8]) -> bool {
    let data = |byte: u8| byte < 0x80;
    match *bytes {
        [status, key, velocity] if matches!(status & 0xF0, 0x80 | 0x90) => {
            data(key) && data(velocity)
        }
        [status, controller, 0]
            if status & 0xF0 == CONTROL && matches!(controller, ALL_NOTES_OFF | ALL_SOUND_OFF) =>
        {
            true
        }
        [status, controller, value] if status & 0xF0 == CONTROL => {
            matches!(controller, midi::WHEEL_CC | midi::PEDAL_CC) && data(value)
        }
        [status, low, high] if status & 0xF0 == BEND => data(low) && data(high),
        [status, program] if status & 0xF0 == PROGRAM => data(program),
        _ => false,
    }
}

/// Where the thread's bytes go: the system's MIDI outputs, or a recording.
pub trait MidiSink: Send {
    /// Sends `bytes` to the output port whose stable name is `port`; why not when it cannot.
    fn send(&mut self, port: &str, bytes: &[u8]) -> Result<(), String>;

    /// Keeps what it holds open for `ports` and lets the rest go.
    fn keep(&mut self, ports: &BTreeSet<String>) {
        let _ = ports;
    }

    /// The ports whose connection stopped reaching the device though no send failed (it was
    /// unplugged, and may be back under the same name), each connection let go so the next send
    /// opens it again; none from a sink that cannot tell.
    fn gone(&mut self) -> Vec<String> {
        Vec::new()
    }
}

/// The system's MIDI output ports: a connection to each port in use, opened the first time it
/// is sent to, under this run's client name ([`midi::this_client`], "Dawsome"), so it never shows
/// as an anonymous RtMidi client in the system's lists.
#[derive(Default)]
pub struct SystemOutputs {
    /// Each open connection by its port's stable name, with the full name (sequencer address
    /// included) of the port it went to.
    connections: BTreeMap<String, (String, MidiOutputConnection)>,
    /// When each port that would not open may be tried again.
    retry: BTreeMap<String, Instant>,
    /// When the connections may be checked again for a device that went ([`MidiSink::gone`]).
    next_check: Option<Instant>,
}

impl MidiSink for SystemOutputs {
    fn send(&mut self, port: &str, bytes: &[u8]) -> Result<(), String> {
        if !self.connections.contains_key(port) {
            if self.retry.get(port).is_some_and(|at| Instant::now() < *at) {
                return Err(format!("the MIDI output {port} would not open"));
            }
            match open_output(port) {
                Ok(opened) => {
                    self.retry.remove(port);
                    self.connections.insert(port.to_owned(), opened);
                }
                Err(error) => {
                    self.retry.insert(port.to_owned(), Instant::now() + RETRY);
                    return Err(error);
                }
            }
        }
        let sent = match self.connections.get_mut(port) {
            Some((_, connection)) => connection.send(bytes).map_err(|error| error.to_string()),
            None => Err("no connection".to_owned()),
        };
        if let Err(error) = sent {
            // Opened again on the next send: the device may be back by then.
            self.connections.remove(port);
            return Err(format!("cannot send to the MIDI output {port}: {error}"));
        }
        Ok(())
    }

    fn keep(&mut self, ports: &BTreeSet<String>) {
        self.connections.retain(|port, _| ports.contains(port));
        self.retry.retain(|port, _| ports.contains(port));
    }

    /// At most once per [`RETRY`]: on ALSA a send to a port whose device went still returns Ok,
    /// so the sequencer's client list is read for the devices this client still sends to
    /// ([`midi::dead_outputs`]).
    fn gone(&mut self) -> Vec<String> {
        let now = Instant::now();
        if self.connections.is_empty() || self.next_check.is_some_and(|at| now < at) {
            return Vec::new();
        }
        self.next_check = Some(now + RETRY);
        let dead = midi::dead_outputs(self.connections.values().map(|(name, _)| name.clone()));
        let mut gone = Vec::new();
        self.connections.retain(|port, (name, _)| {
            let alive = !dead.contains(name);
            if !alive {
                gone.push(port.clone());
            }
            alive
        });
        gone
    }
}

/// A connection to the system's output port whose stable name is `port`, with that port's full
/// name; never to a port of this program's own (its MIDI inputs) or an anonymous RtMidi client.
fn open_output(port: &str) -> Result<(String, MidiOutputConnection), String> {
    let failed = |error: String| format!("cannot open the MIDI output {port}: {error}");
    let client = midi::this_client();
    let output = MidiOutput::new(&client).map_err(|error| failed(error.to_string()))?;
    let found = output.ports().into_iter().find_map(|candidate| {
        let name = output.port_name(&candidate).ok()?;
        (!midi::hidden_port(&name) && stable_port_name(&name) == port).then_some((candidate, name))
    });
    let (found, name) = found.ok_or_else(|| failed("it is not there".to_owned()))?;
    let connection = output
        .connect(&found, &client)
        .map_err(|error| failed(error.to_string()))?;
    Ok((name, connection))
}

/// Where the MIDI output ports come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiOutputs {
    /// The system's ports: listed, and opened when a route sends to them. Only the window of a
    /// run nobody scripts reaches them.
    System,
    /// These names stand in for the ports, and what is sent goes to a [`Recording`]: what a
    /// scripted run shows (`--midi-out-ports`) and what tests see. Nothing is ever opened.
    Given(Vec<String>),
}

impl MidiOutputs {
    /// The output ports' names, this program's own and RtMidi's anonymous clients left out.
    pub fn names(&self) -> Result<Vec<String>, String> {
        let names = match self {
            Self::Given(names) => names.clone(),
            Self::System => {
                let output = MidiOutput::new(&midi::this_client())
                    .map_err(|error| format!("MIDI output is off: {error}"))?;
                output
                    .ports()
                    .iter()
                    .filter_map(|port| output.port_name(port).ok())
                    .collect()
            }
        };
        Ok(names
            .into_iter()
            .filter(|name| !midi::hidden_port(name))
            .collect())
    }

    /// What the thread sends into: the system's outputs, or a new recording for given names.
    pub fn sink(&self) -> Box<dyn MidiSink> {
        match self {
            Self::System => Box::new(SystemOutputs::default()),
            Self::Given(_) => Box::new(Recording::default()),
        }
    }
}

/// One message as a recording keeps it: the port it went to and its bytes.
type Sent = (String, Vec<u8>);

/// What was sent, port and bytes, in order: the stand-in for the ports of a scripted run and a
/// test. A clone is the same recording.
#[derive(Debug, Clone, Default)]
pub struct Recording {
    sent: Arc<Mutex<Vec<Sent>>>,
    /// Ports that take nothing (a test's unplugged device).
    #[cfg(test)]
    gone: Arc<Mutex<BTreeSet<String>>>,
}

impl MidiSink for Recording {
    fn send(&mut self, port: &str, bytes: &[u8]) -> Result<(), String> {
        #[cfg(test)]
        if lock(&self.gone).contains(port) {
            return Err(format!("{port} is unplugged"));
        }
        let mut sent = lock(&self.sent);
        if sent.len() < RECORDING_CAPACITY {
            sent.push((port.to_owned(), bytes.to_vec()));
        }
        Ok(())
    }
}

#[cfg(test)]
impl Recording {
    /// Takes what was sent so far, each message as its port and its bytes in hex.
    pub fn take(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *lock(&self.sent))
            .into_iter()
            .map(|(port, bytes)| {
                let hex: Vec<String> = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
                (port, hex.join(" "))
            })
            .collect()
    }

    /// From now on `port` takes nothing, as an unplugged device; or takes all again.
    pub fn unplug(&self, port: &str, gone: bool) {
        let mut ports = lock(&self.gone);
        if gone {
            ports.insert(port.to_owned());
        } else {
            ports.remove(port);
        }
    }
}

/// The lock, whatever a thread that panicked while holding it left in it: a list of what was
/// sent is never half written.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the thread counts.
#[derive(Debug, Default)]
pub struct Counts {
    /// Messages sent at once because their time was unknown (no stream plays) or past.
    late: AtomicU64,
    /// Messages [`Sending::send_checked`] refused.
    refused: AtomicU64,
    /// Sends a port did not take.
    failed: AtomicU64,
}

impl Counts {
    pub fn late(&self) -> u64 {
        self.late.load(Ordering::Relaxed)
    }

    pub fn refused(&self) -> u64 {
        self.refused.load(Ordering::Relaxed)
    }

    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }
}

/// Everything the thread does but wait: the routes, the sink and what was lost.
struct Sending {
    sink: Box<dyn MidiSink>,
    routes: Routes,
    /// Ports a send failed on, or whose device went ([`MidiSink::gone`]). The notes sounding
    /// there may hang, so once such a port takes bytes again its routes get all notes off before
    /// anything else.
    lost: BTreeSet<String>,
    /// The player's lost note offs, as last seen.
    lost_offs: u64,
    counts: Arc<Counts>,
}

impl Sending {
    fn new(sink: Box<dyn MidiSink>, counts: Arc<Counts>) -> Self {
        Self {
            sink,
            routes: Routes::new(),
            lost: BTreeSet::new(),
            lost_offs: 0,
            counts,
        }
    }

    /// The routes are `routes` from now on: each place an old route leaves gets all notes off
    /// (every old one when `opened`, a document that replaces the last), and each route whose
    /// program is set gets it when the program or the place is new to it (every one when
    /// `opened`).
    fn route(&mut self, routes: Routes, opened: bool) {
        let left: BTreeSet<(String, u8)> = self
            .routes
            .iter()
            .filter(|(id, old)| {
                opened || routes.get(id).is_none_or(|new| new.place() != old.place())
            })
            .map(|(_, old)| old.place())
            .collect();
        for (port, channel) in &left {
            self.all_notes_off(port, *channel);
        }
        for (id, new) in &routes {
            let Some(program) = new.program else {
                continue;
            };
            let kept = !opened && self.routes.get(id).is_some_and(|old| old == new);
            if !kept {
                self.send(&new.port, &[PROGRAM | new.channel, program]);
            }
        }
        self.routes = routes;
        let ports: BTreeSet<String> = self
            .routes
            .values()
            .map(|route| route.port.clone())
            .collect();
        self.lost.retain(|port| ports.contains(port));
        self.sink.keep(&ports);
    }

    /// Sends `event` to its channel's port; nothing when the channel has no route (any more).
    fn event(&mut self, event: &MidiOutEvent) {
        if let Some(port) = self
            .routes
            .get(&event.channel)
            .map(|route| route.port.clone())
        {
            self.send(&port, event.bytes());
        }
    }

    /// The player has lost `total` note offs so far: when that is more than before, every
    /// route gets all notes off, as the notes that lost them cannot be told apart.
    fn lost_offs(&mut self, total: u64) {
        if total <= self.lost_offs {
            return;
        }
        self.lost_offs = total;
        let places: BTreeSet<(String, u8)> = self.routes.values().map(Route::place).collect();
        for (port, channel) in &places {
            self.all_notes_off(port, *channel);
        }
    }

    /// All notes off, then all sound off, on `channel` of `port`.
    fn all_notes_off(&mut self, port: &str, channel: u8) {
        for controller in [ALL_NOTES_OFF, ALL_SOUND_OFF] {
            self.send_checked(port, &[CONTROL | channel, controller, 0]);
        }
    }

    /// Sends `bytes` to `port`; a port lost before gets its routes' all notes off first, and
    /// takes nothing while it does not take those.
    fn send(&mut self, port: &str, bytes: &[u8]) {
        // A port whose device went took bytes without an error and sent them nowhere: as lost
        // as one a send failed on.
        self.lost.extend(self.sink.gone());
        if self.lost.contains(port) && !self.recover(port) {
            return;
        }
        self.send_checked(port, bytes);
    }

    /// All notes off on each MIDI channel of `port`'s routes, for a port lost before; whether
    /// it took them, and so is lost no more.
    fn recover(&mut self, port: &str) -> bool {
        let channels: BTreeSet<u8> = self
            .routes
            .values()
            .filter(|route| route.port == port)
            .map(|route| route.channel)
            .collect();
        for channel in channels {
            for controller in [ALL_NOTES_OFF, ALL_SOUND_OFF] {
                if !self.send_checked(port, &[CONTROL | channel, controller, 0]) {
                    return false;
                }
            }
        }
        self.lost.remove(port);
        true
    }

    /// The one way bytes reach a MIDI output: only what [`allowed`] passes goes to the sink;
    /// the rest is refused and counted. A port that does not take the bytes is counted and
    /// marked lost. Returns whether the port took them.
    ///
    /// Why one choke point for every byte: the instrument on the other end has no spare and
    /// takes messages that can damage it for good (see [`allowed`]); a check at each place that
    /// makes bytes would miss the next place someone adds.
    fn send_checked(&mut self, port: &str, bytes: &[u8]) -> bool {
        if !allowed(bytes) {
            self.counts.refused.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        match self.sink.send(port, bytes) {
            Ok(()) => true,
            Err(_) => {
                self.counts.failed.fetch_add(1, Ordering::Relaxed);
                self.lost.insert(port.to_owned());
                false
            }
        }
    }
}

/// What the window tells the thread.
enum Command {
    /// A document's session opened: its player's ring and its channels' routes.
    Open(Option<MidiOutReader>, Routes),
    /// The channels' routes after a change to the document.
    Routes(Routes),
    /// The clock the output stream publishes, or none while no stream plays.
    Clock(Option<Arc<PlaybackClock>>),
    /// Answers once everything told and read so far is sent (or waits for its time).
    #[cfg(test)]
    Sync(mpsc::Sender<()>),
    Quit,
}

/// The thread's own state: the ring, the clock, and the messages waiting for their time.
struct Worker {
    commands: Receiver<Command>,
    sending: Sending,
    reader: Option<MidiOutReader>,
    clock: Option<Arc<PlaybackClock>>,
    /// Messages read and not yet due, each with the time it is due on the clock.
    waiting: VecDeque<(i128, MidiOutEvent)>,
    /// What the last read of the ring took.
    read: Vec<MidiOutEvent>,
}

impl Worker {
    fn run(mut self) {
        loop {
            let command = match self.pump() {
                Some(wait) => match self.commands.recv_timeout(wait) {
                    Ok(command) => command,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => Command::Quit,
                },
                None => self.commands.recv().unwrap_or(Command::Quit),
            };
            if !self.obey(command) {
                return;
            }
        }
    }

    /// Carries out `command`; `false` once the thread is to end.
    fn obey(&mut self, command: Command) -> bool {
        match command {
            Command::Open(reader, routes) => {
                // The last document's messages never go: its routes end now.
                self.waiting.clear();
                self.reader = reader;
                self.sending.lost_offs = 0;
                self.sending.route(routes, true);
            }
            Command::Routes(routes) => {
                self.pump();
                self.sending.route(routes, false);
            }
            Command::Clock(clock) => self.clock = clock,
            #[cfg(test)]
            Command::Sync(answer) => {
                self.pump();
                // The window may have stopped waiting; nothing is owed to it then.
                let _ = answer.send(());
            }
            Command::Quit => {
                self.waiting.clear();
                self.sending.route(Routes::new(), false);
                return false;
            }
        }
        true
    }

    /// Reads the ring and sends every message that is due; how long until it should look again,
    /// or `None` when only a command can bring anything to send.
    fn pump(&mut self) -> Option<Duration> {
        if let Some(reader) = &mut self.reader {
            reader.read_into(&mut self.read);
        }
        let clock = self.clock.clone();
        for event in std::mem::take(&mut self.read) {
            let due = clock.as_ref().and_then(|clock| clock.time_of(event.frame));
            let now = clock.as_ref().map(|clock| clock.now());
            match (due, now) {
                (Some(due), Some(now)) if due >= now => self.waiting.push_back((due, event)),
                _ => {
                    // Its time is unknown or past: it goes now, after what waits before it.
                    self.flush();
                    self.sending.counts.late.fetch_add(1, Ordering::Relaxed);
                    self.sending.event(&event);
                }
            }
        }
        let now = clock.as_ref().map(|clock| clock.now());
        while let Some(&(due, event)) = self.waiting.front() {
            if now.is_some_and(|now| due > now) {
                break;
            }
            self.waiting.pop_front();
            self.sending.event(&event);
        }
        if let Some(lost) = self.reader.as_ref().map(MidiOutReader::lost_offs) {
            self.sending.lost_offs(lost);
        }
        match (self.waiting.front(), now) {
            (Some(&(due, _)), Some(now)) => {
                let nanos = u64::try_from(due - now).unwrap_or(0);
                Some(POLL.min(Duration::from_nanos(nanos)))
            }
            _ if self.reader.is_some() && !self.sending.routes.is_empty() => Some(POLL),
            _ => None,
        }
    }

    /// Sends every waiting message now, in order.
    fn flush(&mut self) {
        while let Some((_, event)) = self.waiting.pop_front() {
            self.sending.event(&event);
        }
    }
}

/// The MIDI-out thread and the window's line to it. Dropping it ends the thread, every route's
/// notes ended first.
pub struct MidiOutThread {
    commands: mpsc::Sender<Command>,
    thread: Option<JoinHandle<()>>,
    counts: Arc<Counts>,
}

impl MidiOutThread {
    /// Starts the thread, sending into `sink`; why not when it cannot start.
    pub fn start(sink: Box<dyn MidiSink>) -> Result<Self, String> {
        let (commands, received) = mpsc::channel();
        let counts = Arc::new(Counts::default());
        let worker = Worker {
            commands: received,
            sending: Sending::new(sink, Arc::clone(&counts)),
            reader: None,
            clock: None,
            waiting: VecDeque::new(),
            read: Vec::new(),
        };
        let thread = std::thread::Builder::new()
            .name("dawsome-midi-out".to_owned())
            .spawn(move || worker.run())
            .map_err(|error| format!("MIDI out is off: {error}"))?;
        Ok(Self {
            commands,
            thread: Some(thread),
            counts,
        })
    }

    /// A document's session opened: the thread reads `reader` (its player's ring) from now on
    /// and its channels' routes are `routes`; every route of the last document ends, and every
    /// program set is sent.
    pub fn open(&self, reader: Option<MidiOutReader>, routes: Routes) {
        self.tell(Command::Open(reader, routes));
    }

    /// The channels' routes after a change to the document.
    pub fn routes(&self, routes: Routes) {
        self.tell(Command::Routes(routes));
    }

    /// The clock of the output stream playing the player, or none.
    pub fn clock(&self, clock: Option<Arc<PlaybackClock>>) {
        self.tell(Command::Clock(clock));
    }

    /// What the thread has counted.
    pub fn counts(&self) -> &Counts {
        &self.counts
    }

    /// Waits until the thread has carried out everything told so far and sent what it read
    /// that was due.
    #[cfg(test)]
    pub fn sync(&self) {
        let (answer, answered) = mpsc::channel();
        self.tell(Command::Sync(answer));
        // A thread that ended answers nothing; there is nothing to wait for then.
        let _ = answered.recv();
    }

    /// Ends the thread, every route's notes ended first, and waits for it.
    pub fn quit(&mut self) {
        self.tell(Command::Quit);
        if let Some(thread) = self.thread.take() {
            // A thread that panicked has nothing left to send.
            let _ = thread.join();
        }
    }

    fn tell(&self, command: Command) {
        // A thread that ended (quit, or panicked) takes nothing more, and nothing is owed to it.
        let _ = self.commands.send(command);
    }
}

impl Drop for MidiOutThread {
    fn drop(&mut self) {
        self.quit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNTH: &str = "Synth:Synth MIDI 1";

    fn route(port: &str, channel: u8, program: Option<u8>) -> Route {
        Route {
            port: port.to_owned(),
            channel,
            program,
        }
    }

    fn sending() -> (Sending, Recording) {
        let recording = Recording::default();
        let counts = Arc::new(Counts::default());
        (Sending::new(Box::new(recording.clone()), counts), recording)
    }

    fn sent(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(port, bytes)| ((*port).to_owned(), (*bytes).to_owned()))
            .collect()
    }

    // By hand from the MIDI 1.0 status bytes: 8n/9n note off/on with two data bytes, Bn 7B 00 and
    // Bn 78 00 all notes off and all sound off, Cn pp a program change, Bn 01 vv the mod wheel,
    // Bn 40 vv the sustain pedal, En ll mm pitch bend; everything else is refused, counted, and
    // never reaches the port.
    #[test]
    fn send_checked_passes_only_notes_all_notes_off_program_changes_and_the_players_controls() {
        let (mut sending, recording) = sending();
        let passed: [&[u8]; 11] = [
            &[0x90, 60, 100],
            &[0x8F, 127, 64],
            &[0x9F, 0, 1],
            &[0xB0, 123, 0],
            &[0xB3, 120, 0],
            &[0xC0, 5],
            &[0xCF, 127],
            &[0xB0, 1, 100],
            &[0xB2, 64, 127],
            &[0xE0, 0, 64],
            &[0xEF, 0x7F, 0x7F],
        ];
        for bytes in passed {
            assert!(sending.send_checked(SYNTH, bytes), "{bytes:02X?} passes");
        }
        let refused: [&[u8]; 18] = [
            &[0xF0, 0x7F, 0x73, 0xF7],
            &[0xF0, 0x00, 0x20, 0x33, 0x7F, 0x77, 0xF7],
            &[0xF0],
            &[0xB0, 7, 100],
            &[0xB0, 0, 0],
            &[0xB0, 123, 1],
            &[0xB0, 121, 0],
            &[0x90, 60],
            &[0x80, 60],
            &[0x90, 60, 100, 0],
            &[0x90, 0x80, 100],
            &[0xC0, 0x80],
            &[0xB0, 1, 0x80],
            &[0xB0, 2, 100],
            &[0xE0, 0x80, 0],
            &[0xE0, 0],
            &[0xE0, 0, 64, 0],
            &[],
        ];
        for bytes in refused {
            assert!(
                !sending.send_checked(SYNTH, bytes),
                "{bytes:02X?} is refused"
            );
        }
        assert_eq!(sending.counts.refused(), 18);
        assert_eq!(
            recording.take(),
            sent(&[
                (SYNTH, "90 3C 64"),
                (SYNTH, "8F 7F 40"),
                (SYNTH, "9F 00 01"),
                (SYNTH, "B0 7B 00"),
                (SYNTH, "B3 78 00"),
                (SYNTH, "C0 05"),
                (SYNTH, "CF 7F"),
                (SYNTH, "B0 01 64"),
                (SYNTH, "B2 40 7F"),
                (SYNTH, "E0 00 40"),
                (SYNTH, "EF 7F 7F"),
            ])
        );
    }

    // All notes off (CC 123, then CC 120) on the old place of a route whose MIDI channel or port
    // changes or that goes, once per place; a program change only when it or its place is new to
    // its route, and for every route when a document opens, which ends every old route first.
    #[test]
    fn routes_end_their_old_places_and_set_new_programs() {
        let (mut sending, recording) = sending();
        let (a, b) = (ChannelId(1), ChannelId(2));
        let routes = |list: &[(ChannelId, Route)]| list.iter().cloned().collect::<Routes>();
        sending.route(
            routes(&[(a, route(SYNTH, 0, Some(4))), (b, route(SYNTH, 2, None))]),
            true,
        );
        assert_eq!(recording.take(), sent(&[(SYNTH, "C0 04")]), "on open");
        sending.route(
            routes(&[(a, route(SYNTH, 0, Some(4))), (b, route(SYNTH, 2, None))]),
            false,
        );
        assert!(
            recording.take().is_empty(),
            "the same routes again send nothing"
        );
        sending.route(
            routes(&[(a, route(SYNTH, 1, Some(4))), (b, route(SYNTH, 2, None))]),
            false,
        );
        assert_eq!(
            recording.take(),
            sent(&[(SYNTH, "B0 7B 00"), (SYNTH, "B0 78 00"), (SYNTH, "C1 04")]),
            "a MIDI channel changed"
        );
        sending.route(
            routes(&[(a, route(SYNTH, 1, Some(9))), (b, route("Other", 2, None))]),
            false,
        );
        assert_eq!(
            recording.take(),
            sent(&[(SYNTH, "B2 7B 00"), (SYNTH, "B2 78 00"), (SYNTH, "C1 09")]),
            "a port changed, a program changed"
        );
        sending.route(routes(&[(b, route("Other", 2, None))]), false);
        assert_eq!(
            recording.take(),
            sent(&[(SYNTH, "B1 7B 00"), (SYNTH, "B1 78 00")]),
            "a route gone"
        );
        sending.route(routes(&[(b, route("Other", 2, Some(0)))]), true);
        assert_eq!(
            recording.take(),
            sent(&[
                ("Other", "B2 7B 00"),
                ("Other", "B2 78 00"),
                ("Other", "C2 00")
            ]),
            "a document opened on the same place"
        );
    }

    // A port that did not take a message gets its routes' all notes off before the next message
    // it takes; while it takes nothing, nothing is counted twice into it.
    #[test]
    fn a_port_back_after_it_was_lost_ends_its_notes_first() {
        let (mut sending, recording) = sending();
        let routes: Routes = [(ChannelId(1), route(SYNTH, 0, None))]
            .into_iter()
            .collect();
        sending.route(routes, true);
        let on = note(ChannelId(1));
        sending.event(&on);
        recording.unplug(SYNTH, true);
        sending.event(&on);
        sending.event(&on);
        assert_eq!(
            sending.counts.failed(),
            2,
            "the note, then the first CC 123 of its retry"
        );
        recording.unplug(SYNTH, false);
        sending.event(&on);
        assert_eq!(
            recording.take(),
            sent(&[
                (SYNTH, "90 3C 64"),
                (SYNTH, "B0 7B 00"),
                (SYNTH, "B0 78 00"),
                (SYNTH, "90 3C 64"),
            ])
        );
    }

    // On ALSA a send to a port whose device went (unplugged,
    // then plugged back) returns Ok and goes nowhere, so the port was never lost and stayed silent
    // until a restart. Here the port is the test's own virtual one (never a device of the user's),
    // taken away and made again under the same name: once the connections are checked again (at
    // most once per RETRY), the second one gets all notes off, then the note.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_port_that_went_and_came_back_ends_its_notes_then_plays_again() {
        use midir::os::unix::VirtualInput;
        use midir::{MidiInput, MidiInputConnection};
        type Heard = Arc<Mutex<Vec<Vec<u8>>>>;
        let client = format!("DawsomeTestOut{}", std::process::id());
        let make = |heard: &Heard| -> Option<MidiInputConnection<()>> {
            let heard = Arc::clone(heard);
            let input = MidiInput::new(&client).ok()?;
            let keep = move |_: u64, bytes: &[u8], _: &mut ()| lock(&heard).push(bytes.to_vec());
            input.create_virtual(&client, keep, ()).ok()
        };
        // Delivery runs on the port's own thread: waits up to 2 s for `count` messages.
        let wait_for = |heard: &Heard, count: usize| {
            let until = Instant::now() + Duration::from_secs(2);
            while lock(heard).len() < count && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            lock(heard).clone()
        };
        let (first, second): (Heard, Heard) = Default::default();
        let Some(life) = make(&first) else {
            eprintln!("no ALSA sequencer: the unplug test cannot run here");
            return;
        };
        let port = format!("{client}:{client}");
        let counts = Arc::new(Counts::default());
        let mut sending = Sending::new(Box::new(SystemOutputs::default()), counts);
        let routes: Routes = [(ChannelId(1), route(&port, 0, None))]
            .into_iter()
            .collect();
        sending.route(routes, true);
        let on = [0x90, 60, 100];
        // The first send opens the connection, the second checks it: still there.
        sending.send(&port, &on);
        sending.send(&port, &on);
        assert_eq!(wait_for(&first, 2), [on.to_vec(), on.to_vec()]);
        drop(life);
        let life = make(&second).expect("the port made again");
        std::thread::sleep(RETRY + Duration::from_millis(50));
        sending.send(&port, &on);
        assert_eq!(
            wait_for(&second, 3),
            [vec![0xB0, 123, 0], vec![0xB0, 120, 0], on.to_vec()],
            "all notes off, then the note"
        );
        drop(life);
    }

    // When the player has lost note offs (its ring was full to the last slot), every route's
    // place gets all notes off once, and again only when it loses more.
    #[test]
    fn lost_note_offs_end_every_routes_notes() {
        let (mut sending, recording) = sending();
        let routes: Routes = [
            (ChannelId(1), route(SYNTH, 0, None)),
            (ChannelId(2), route(SYNTH, 0, None)),
            (ChannelId(3), route("Other", 5, None)),
        ]
        .into_iter()
        .collect();
        sending.route(routes, true);
        sending.lost_offs(0);
        assert!(recording.take().is_empty());
        sending.lost_offs(2);
        let ended = sent(&[
            ("Other", "B5 7B 00"),
            ("Other", "B5 78 00"),
            (SYNTH, "B0 7B 00"),
            (SYNTH, "B0 78 00"),
        ]);
        assert_eq!(recording.take(), ended);
        sending.lost_offs(2);
        assert!(recording.take().is_empty(), "nothing more was lost");
        sending.lost_offs(3);
        assert_eq!(recording.take(), ended);
    }

    /// A note on of key 60 at velocity 100/128 on MIDI channel 1 for `channel`, as the player
    /// sends one: through the engine's own ring, the only maker of messages.
    fn note(channel: ChannelId) -> MidiOutEvent {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p19_echo.json");
        let opened = crate::project_file::open(&path).expect("p19_echo opens");
        let (mut session, mut player) =
            crate::session::Session::open(opened.project, opened.samples_dir, false)
                .expect("opens");
        let mut reader = session.take_midi_out().expect("the ring");
        session.midi_inbox().push_to(
            channel,
            dawsome_engine::realtime::LiveNote::On {
                key: 60,
                velocity: 100.0 / 128.0,
            },
        );
        player.process(&mut [[0.0; 2]; 16]);
        reader.pop().expect("the note went out")
    }

    // The thread: what the player sends goes to its channel's port, and with no stream playing
    // (no clock) each message goes at once and is counted late. Quitting ends every route's
    // notes.
    #[test]
    fn the_thread_sends_the_players_messages_and_ends_every_route_on_quit() {
        let recording = Recording::default();
        let mut thread = MidiOutThread::start(Box::new(recording.clone())).expect("starts");
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/p19_echo.json");
        let opened = crate::project_file::open(&path).expect("p19_echo opens");
        let routes = routes(&opened.project);
        let (mut session, mut player) =
            crate::session::Session::open(opened.project, opened.samples_dir, false)
                .expect("opens");
        thread.open(session.take_midi_out(), routes);
        session.midi_inbox().push_to(
            ChannelId(3),
            dawsome_engine::realtime::LiveNote::On {
                key: 67,
                velocity: 70.0 / 128.0,
            },
        );
        player.process(&mut [[0.0; 2]; 16]);
        thread.sync();
        assert_eq!(recording.take(), sent(&[("Echo:Echo MIDI 1", "92 43 46")]));
        assert_eq!(thread.counts().late(), 1, "no stream plays");
        thread.quit();
        assert_eq!(
            recording.take(),
            sent(&[
                ("Echo:Echo MIDI 1", "B2 7B 00"),
                ("Echo:Echo MIDI 1", "B2 78 00"),
                ("Linked:Linked MIDI 1", "B1 7B 00"),
                ("Linked:Linked MIDI 1", "B1 78 00"),
                (SYNTH, "B0 7B 00"),
                (SYNTH, "B0 78 00"),
            ])
        );
    }

    // The ports listed for the MIDI OUT picker leave out this program's own client (its MIDI
    // inputs show as outputs to it) and anonymous RtMidi clients.
    #[test]
    fn the_listed_outputs_leave_out_our_own_and_rtmidi_clients() {
        let own = format!("{}:{} 130:0", midi::this_client(), midi::this_client());
        let outputs = MidiOutputs::Given(vec![
            SYNTH.to_owned(),
            own,
            "RtMidiOut Client:RtMidi output 128:0".to_owned(),
            "Midi Through:Midi Through Port-0 14:0".to_owned(),
        ]);
        assert_eq!(
            outputs.names(),
            Ok(vec![
                SYNTH.to_owned(),
                "Midi Through:Midi Through Port-0 14:0".to_owned()
            ])
        );
    }
}
