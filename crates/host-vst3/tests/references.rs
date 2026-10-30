//! 12 (references). The VST3 host's reference counts, against the test module's own count of its
//! live COM objects and of `ModuleEntry`/`ModuleExit` (read through the same loaded image the
//! host loads: a second `dlopen` of one path is that image). A test binary of its own, with one
//! test, because those counts are the process's, and another test loading the module at the
//! same time would move them.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]
#![deny(clippy::multiple_unsafe_ops_per_block)]

use std::convert::Infallible;

use dawsome_engine::compile::Compiler;
use dawsome_engine::dsp::sampler::SampleBuffer;
use dawsome_engine::edit::Touched;
use dawsome_engine::plan::PlayMode;
use dawsome_engine::processor::{ProcessorHost, Registry};
use dawsome_engine::project::{
    ChannelId, Effect, EffectId, InsertId, PatternId, ProcessorOwner, ProcessorRef, Project,
};
use dawsome_engine::realtime;
use dawsome_engine::transport::Control;
use dawsome_host_common::catalogue::{self, Catalogue, FileRecord, FileStatus, PluginFormat};
use dawsome_host_vst3::{host_objects, scan, Vst3Host};

#[path = "../../../tests/support/test_plugins.rs"]
mod test_plugins;

const P27_SINE: &str = include_str!("../../../tests/fixtures/p27_vst3_sine.json");
const MODULE: &str = "Dawsome Test VST3.vst3";
const GAIN: &str = "vst3:446177736F6D65546573744761696E31";

fn catalogue() -> Catalogue {
    let module = test_plugins::test_plugins_vst3().join(MODULE);
    let plugins = scan::describe_module(&module, || {}).expect("the test module loads");
    let (size, mtime) =
        catalogue::module_stamp(PluginFormat::Vst3, &module).expect("the module is there");
    Catalogue {
        files: vec![FileRecord {
            file: module,
            format: PluginFormat::Vst3,
            size,
            mtime,
            status: FileStatus::Ok,
            reason: None,
            plugins,
        }],
        ..Catalogue::default()
    }
}

/// The test module's counters.
struct Counts {
    library: libloading::Library,
}

impl Counts {
    fn open() -> Self {
        let binary = PluginFormat::Vst3.binary(&test_plugins::test_plugins_vst3().join(MODULE));
        // SAFETY: the test module, our own code; opening it again runs no entry (only the host
        // calls `ModuleEntry`).
        let library = unsafe { libloading::Library::new(binary) }.expect("opens");
        Self { library }
    }

    fn live(&self) -> isize {
        // SAFETY: the module exports `dawsome_test_vst3_live_objects() -> isize`, a read.
        unsafe {
            let read = self
                .library
                .get::<extern "C" fn() -> isize>(b"dawsome_test_vst3_live_objects\0")
                .expect("exported");
            read()
        }
    }

    fn entered(&self) -> i32 {
        // SAFETY: the module exports `dawsome_test_vst3_module_entered() -> i32`, a read.
        unsafe {
            let read = self
                .library
                .get::<extern "C" fn() -> i32>(b"dawsome_test_vst3_module_entered\0")
                .expect("exported");
            read()
        }
    }
}

/// The sine fixture with the gain at 0.5 in the master's first slot.
fn project() -> Project {
    let mut project = Project::load_json(P27_SINE).expect("fixture loads");
    let mut gain = ProcessorRef::plugin(GAIN, "Dawsome Test Gain");
    gain.state = None;
    project.mixer.inserts[usize::from(InsertId::MASTER.0)]
        .effects
        .push(Effect {
            id: EffectId(1),
            slot: 0,
            processor: gain,
        });
    project.mixer.next_effect_id = EffectId(2);
    project.validate().expect("valid");
    project
}

// A hundred rounds of making the sine (component, controller, their connection and its
// messages) and the gain, playing them, and letting them go end with no object of the module
// alive but its factory, and none of the host's but the module's context: every reference the
// host took is given back, and every one the plugin took of ours too. The module stays entered
// while the host holds it, between the rounds as well, and its exit comes when the host goes,
// after which nothing of it is alive.
#[test]
fn a_hundred_instances_made_and_let_go_leave_nothing_alive() {
    let counts = Counts::open();
    let catalogue = catalogue();
    assert_eq!(
        (counts.live(), counts.entered()),
        (0, 0),
        "the scan's own load undone"
    );
    let ours = host_objects();
    let project = project();
    {
        let host = Vst3Host::new(catalogue);
        for round in 0..100 {
            let session = host.session();
            let mut registry = Registry::native();
            registry.add_host(Box::new(session.clone()));
            let mut compiler = Compiler::with_registry(registry);
            let silent = |_: &_| Ok::<_, Infallible>(SampleBuffer::from(vec![1.0]));
            let update = compiler
                .compile(
                    &project,
                    &Touched::default(),
                    PlayMode::Pattern(PatternId(1)),
                    silent,
                )
                .expect("compiles");
            assert!(compiler.take_notices().is_empty(), "round {round}");
            let (mut link, mut player) = realtime::link(update, project.tempo);
            link.send(Control::Play).expect("room");
            let mut buffer = [[0.0f32; 2]; 512];
            for _ in 0..14 {
                player.process(&mut buffer);
            }
            assert!(
                counts.live() >= 4,
                "the factory and three objects, round {round}"
            );
            drop(player);
            drop((compiler, link, session));
            host.tick();
            assert_eq!(host.instances(), 0, "round {round}");
            assert_eq!(counts.live(), 1, "only the factory, round {round}");
            assert_eq!(
                host_objects() - ours,
                1,
                "only the module's context, round {round}"
            );
            assert_eq!(counts.entered(), 1, "still loaded, round {round}");
        }
        assert_eq!(host.created(), 200);
        let owner = ProcessorOwner::Channel(ChannelId(1));
        assert!(host.value(owner, 0).is_none(), "every instance gone");
    }
    assert_eq!(counts.entered(), 0, "the host's going called ModuleExit");
    assert_eq!(counts.live(), 0, "the factory released");
    assert_eq!(host_objects(), ours, "every object of the host's gone");
}
