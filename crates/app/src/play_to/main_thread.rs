//! The plugins of a render on a worker thread, made and served on the window's thread: the
//! window's export. Why: a plugin's main thread is its process's one, the thread of the window,
//! and some plugins hold to that whatever host made them. Surge XT (JUCE) keeps one message
//! thread per process, the thread that made its first instance (the window's own, for a project
//! that plays it); an instance made and pumped on the export's worker then waited in its timer
//! for that thread's lock, which nothing there gives, and the export stopped for good (build
//! d5f9f02: Export WAV and Export stems at 23 %).
//!
//! So the export's instances live in hosts of their own made on the window's thread
//! ([`RenderHosts`]); the worker's compiler reaches them through the hosts [`MainThread::hosts`]
//! gives, each call a job the window runs on its plugin tick while the worker waits for the
//! answer. The render runs on the worker, the plugins' audio thread, and the window serves
//! their timers, file descriptors and callbacks. The command line and `--play-to` have one
//! host's thread only, the one rendering, and pump there as before.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use dawsome_engine::processor::{
    Captured, Editor, HostError, HostNotice, PluginParam, Processor, ProcessorHost,
};
use dawsome_engine::project::{ProcessorOwner, ProcessorRef, Project};
use dawsome_host_clap::{Catalogue, ClapHost, RenderPump};
use dawsome_host_vst3::Vst3Host;

/// Work for the window's thread, sent by a render's worker.
type Job = Box<dyn FnOnce(&mut Hosts) + Send>;

/// The render's hosts on the window's thread, and a handle of each for every session the worker
/// has open: an instance goes once its session's handles are gone and its audio half is back.
struct Hosts {
    clap: ClapHost,
    vst3: Vst3Host,
    sessions: BTreeMap<u64, (ClapHost, Vst3Host)>,
}

impl Hosts {
    /// The host of `format` for `session`, while it is open.
    fn host(&self, session: u64, format: &str) -> Option<&dyn ProcessorHost> {
        let (clap, vst3) = self.sessions.get(&session)?;
        match format {
            dawsome_host_clap::FORMAT => Some(clap),
            dawsome_host_vst3::FORMAT => Some(vst3),
            _ => None,
        }
    }
}

/// A render's plugin hosts, kept on the window's thread: what [`RenderHosts::serve`] runs on
/// each plugin tick, until the worker is done with them and every instance they made is gone.
pub struct RenderHosts {
    hosts: Hosts,
    jobs: mpsc::Receiver<Job>,
    pump: RenderPump,
    /// Every handle the worker held is gone: its processors with them.
    worker_done: bool,
    /// Each plugin failure heard, once, as the export names it.
    failures: Rc<RefCell<Vec<String>>>,
}

impl RenderHosts {
    /// Hosts for a render of `project`'s plugins, found in `catalogue` (the window's: the export
    /// plays what the window plays), made on this thread; and the handle the worker reaches them
    /// through.
    pub fn new(project: &Project, catalogue: &Catalogue) -> (Self, MainThread) {
        let clap = ClapHost::new(catalogue.clone());
        let vst3 = Vst3Host::new(catalogue.clone());
        let pump = RenderPump::of(
            vec![Box::new(clap.clone()), Box::new(vst3.clone())],
            project,
        );
        let (sender, jobs) = mpsc::channel();
        let hosts = Self {
            hosts: Hosts {
                clap,
                vst3,
                sessions: BTreeMap::new(),
            },
            jobs,
            pump,
            worker_done: false,
            failures: Rc::default(),
        };
        let main = MainThread {
            jobs: sender,
            next_session: Arc::new(AtomicU64::new(0)),
        };
        (hosts, main)
    }

    /// The failures heard so far, which later ones join.
    pub fn failures(&self) -> Rc<RefCell<Vec<String>>> {
        Rc::clone(&self.failures)
    }

    /// Runs the jobs the worker has sent, in order, then the plugins' main-thread work, keeping
    /// each failure it hears.
    pub fn serve(&mut self) {
        loop {
            match self.jobs.try_recv() {
                Ok(job) => job(&mut self.hosts),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker_done = true;
                    break;
                }
            }
        }
        let heard = self.pump.pump();
        self.failures.borrow_mut().extend(heard);
    }

    /// Whether it may go: the worker let go of it, and every instance it made is gone.
    pub fn is_spent(&self) -> bool {
        self.worker_done && !self.hosts.clap.has_instances() && !self.hosts.vst3.has_instances()
    }
}

/// The worker's way to a render's hosts on the window's thread. Every call waits until the
/// window runs it: never use it on the window's thread.
#[derive(Clone)]
pub struct MainThread {
    jobs: mpsc::Sender<Job>,
    next_session: Arc<AtomicU64>,
}

impl MainThread {
    /// A host of each plugin format for one session's compiler; the instances it makes go once
    /// both hosts are dropped and their processors are.
    pub fn hosts(&self) -> Vec<Box<dyn ProcessorHost>> {
        let session = self.next_session.fetch_add(1, Ordering::Relaxed);
        self.send(move |hosts| {
            let handles = (hosts.clap.session(), hosts.vst3.session());
            hosts.sessions.insert(session, handles);
        });
        let scope = Rc::new(Scope {
            main: self.clone(),
            session,
        });
        [dawsome_host_clap::FORMAT, dawsome_host_vst3::FORMAT]
            .into_iter()
            .map(|format| {
                Box::new(Forwarded {
                    format,
                    scope: Rc::clone(&scope),
                }) as Box<dyn ProcessorHost>
            })
            .collect()
    }

    /// Sends `job`; false when the window no longer serves these hosts.
    fn send(&self, job: impl FnOnce(&mut Hosts) + Send + 'static) -> bool {
        self.jobs.send(Box::new(job)).is_ok()
    }

    /// Runs `call` on the window's thread and waits for its answer; `None` when the window no
    /// longer serves these hosts.
    fn call<T: Send + 'static>(
        &self,
        call: impl FnOnce(&mut Hosts) -> T + Send + 'static,
    ) -> Option<T> {
        let (answer, answered) = mpsc::channel();
        let sent = self.send(move |hosts| {
            let _ = answer.send(call(hosts));
        });
        if !sent {
            return None;
        }
        answered.recv().ok()
    }
}

/// One session of the worker's: once its hosts are dropped, its handles on the window's thread
/// go, and with them its instances.
struct Scope {
    main: MainThread,
    session: u64,
}

impl Drop for Scope {
    fn drop(&mut self) {
        let session = self.session;
        self.main.send(move |hosts| {
            hosts.sessions.remove(&session);
        });
    }
}

/// A plugin format's host on the window's thread, as the worker's compiler calls it. A render
/// opens no plugin window and pumps nothing here: the window does that.
struct Forwarded {
    format: &'static str,
    scope: Rc<Scope>,
}

impl Forwarded {
    /// Runs `call` with this session's host of this format on the window's thread.
    fn call<T: Send + 'static>(
        &self,
        call: impl FnOnce(&dyn ProcessorHost) -> T + Send + 'static,
    ) -> Option<T> {
        let (format, session) = (self.format, self.scope.session);
        self.scope
            .main
            .call(move |hosts| hosts.host(session, format).map(call))
            .flatten()
    }

    /// Sends `call` to this session's host of this format, without waiting.
    fn send(&self, call: impl FnOnce(&dyn ProcessorHost) + Send + 'static) {
        let (format, session) = (self.format, self.scope.session);
        self.scope.main.send(move |hosts| {
            if let Some(host) = hosts.host(session, format) {
                call(host);
            }
        });
    }
}

/// Why a call found no host: the window has stopped serving this render.
fn not_served() -> HostError {
    HostError::Failed("the window no longer serves this render".into())
}

impl ProcessorHost for Forwarded {
    fn format(&self) -> &'static str {
        self.format
    }

    fn create(
        &self,
        owner: ProcessorOwner,
        reference: &ProcessorRef,
        sample_rate: u32,
    ) -> Result<Box<dyn Processor>, HostError> {
        let reference = reference.clone();
        self.call(move |host| host.create(owner, &reference, sample_rate))
            .unwrap_or_else(|| Err(not_served()))
    }

    fn release(&self, owner: ProcessorOwner) {
        self.send(move |host| host.release(owner));
    }

    fn sync_state(&self, owner: ProcessorOwner, reference: &ProcessorRef) {
        let reference = reference.clone();
        self.send(move |host| host.sync_state(owner, &reference));
    }

    fn params(&self, owner: ProcessorOwner) -> Vec<PluginParam> {
        self.call(move |host| host.params(owner))
            .unwrap_or_default()
    }

    fn value(&self, owner: ProcessorOwner, id: u32) -> Option<f64> {
        self.call(move |host| host.value(owner, id)).flatten()
    }

    fn value_text(&self, owner: ProcessorOwner, id: u32, value: f64) -> Option<String> {
        self.call(move |host| host.value_text(owner, id, value))
            .flatten()
    }

    fn capture_state(&self, owner: ProcessorOwner) -> Result<Captured, HostError> {
        self.call(move |host| host.capture_state(owner))
            .unwrap_or_else(|| Err(not_served()))
    }

    fn latency(&self, owner: ProcessorOwner) -> Option<u32> {
        self.call(move |host| host.latency(owner)).flatten()
    }

    fn open_editor(&self, _owner: ProcessorOwner, _title: &str) -> Result<Editor, HostError> {
        Err(HostError::Failed("a render opens no plugin window".into()))
    }

    fn tick(&self) -> Vec<HostNotice> {
        Vec::new()
    }

    fn has_instances(&self) -> bool {
        false
    }
}
